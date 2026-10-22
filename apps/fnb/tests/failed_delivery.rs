//! BL-FNB-01 (#365), 06 §4.2 rows *Own-rider delivery fails* and *Rider's run
//! is aborted*, §7.1 *Order* row `fail_delivery` and the failure reasons,
//! `WRK.RUL.16`, `WRK.RUL.08`: when the outlet's own rider cannot deliver, the
//! order goes from `dispatched` to `delivery_failed` with the failure reason
//! mapped from wl-delivery's, the customer is told, nothing is collected on
//! cash on delivery and no reattempt is queued. The delivery module is the
//! real wl-delivery with the 06 settings; the rider records the failure in the
//! driver PWA.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::FnbOrderStatus;
use fnb::order::{party, reason, side};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine;
use wl_delivery::DeliveryStatus;
use wl_delivery::hand_over::HandOver;
use wl_kernel::delivery::DeliveryAttemptFailed;

use support::{World, boot_own_riders};

/// One shawarma plate at AED 85.00 (zero-rated: the story's figure).
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "85.00")];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// A prepaid order `dispatched`, for a customer of its own.
async fn prepaid(w: &World) -> (fnb::FnbOrder, wl_delivery::Delivery, wl_delivery::Run) {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    w.dispatched(customer, "online", &PLATE).await
}

/// The titles of the notices the customer was sent.
fn customer_titles(w: &World) -> Vec<String> {
    w.notices
        .of_type(fnb::notices::CUSTOMER_NOTICE)
        .into_iter()
        .filter_map(|r| r.data["title"].as_str().map(str::to_owned))
        .collect()
}

/// AC1: given a `dispatched` prepaid order ORD-501 (AED 85.00), when the rider
/// records *not home*, the order is `delivery_failed` with reason customer
/// unreachable, the customer is told, and no reattempt is queued.
#[renox::test]
async fn not_home_fails_the_prepaid_order_with_customer_unreachable_and_tells_the_customer() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = prepaid(&w).await;
    assert_eq!(order.total, 8_500);
    assert_eq!(order.paid_amount, 8_500);

    w.fail_stop(&delivery, "not_home").await.assert_status(303);

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::DeliveryFailed);
    assert_eq!(after.failure_reason.as_deref(), Some(reason::CUSTOMER_UNREACHABLE));
    assert_eq!(after.delivery_reason.as_deref(), Some("not_home"));
    assert_eq!(after.failure_side.as_deref(), Some(side::CUSTOMER));
    assert_eq!(after.responsible_party.as_deref(), Some(party::CUSTOMER));
    assert_eq!(after.failure_attempt, 1);
    assert!(after.failed_at.is_some());
    // The customer is told, without blame.
    let titles = customer_titles(&w);
    assert!(
        titles
            .iter()
            .any(|title| title == &format!("We could not deliver your order {}", order.number)),
        "{titles:?}"
    );
    // No reattempt: the delivery waits for its food, and a manager cannot
    // queue it again.
    let failed = w.delivery(&delivery).await;
    assert_eq!(failed.status, DeliveryStatus::Attempted);
    assert_eq!(failed.resolution.as_deref(), Some("return"));
    w.post(
        &w.manager,
        &format!("/deliveries/{}/reattempt", delivery.id),
        &[],
    )
    .await
    .assert_status(422);
    assert_eq!(w.delivery(&delivery).await.status, DeliveryStatus::Attempted);
}

/// AC2: given a cash order ORD-502 and the rider records *no cash*, the order
/// is `delivery_failed` and the rider's expected cash does not include
/// ORD-502.
#[renox::test]
async fn no_cash_fails_the_cash_order_and_it_is_left_out_of_the_riders_expected_cash() {
    let w = boot_own_riders().await;
    let customer = w.customer("Khalid Mansoori").await;
    let (order, delivery, run) = w.dispatched(customer, "cash_on_delivery", &PLATE).await;
    let on_the_road = w.delivery(&delivery).await;
    assert_eq!(on_the_road.amount_to_collect, 8_500);

    w.fail_stop(&delivery, "no_cash").await.assert_status(303);

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::DeliveryFailed);
    assert_eq!(after.failure_reason.as_deref(), Some(reason::NO_CASH));
    assert_eq!(after.failure_side.as_deref(), Some(side::CUSTOMER));
    let run = wl_delivery::Run::find_or_404(w.db(), run.id).await.unwrap();
    let expected = wl_delivery::close::cash_expected(w.db(), &run).await.unwrap();
    assert_eq!(expected, 0, "nothing was collected, so nothing is expected");
}

/// AC3: given run R1 aborted and stop ORD-503 whose brand promise ends before
/// the receiving run can arrive, the hand-over is refused, the stop stays on
/// R1, and the abort fails it: ORD-503 is `delivery_failed` with *run aborted*
/// (outlet side, business), and its food is discarded under the abort policy.
#[renox::test]
async fn a_run_aborted_with_no_hand_over_in_time_fails_the_order_with_run_aborted() {
    let w = boot_own_riders().await;
    let (order, delivery, run) = prepaid(&w).await;
    let receiving = w.run_of(&w.other_rider).await;
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/dispatch", receiving.id),
        &[],
    )
    .await
    .assert_ok();
    // The receiving run could arrive 60 minutes after the order was placed;
    // the promise is 45.
    let placed = w.delivery(&delivery).await.placed_at.expect("a placed time");
    let late = (placed + TimeDelta::minutes(60) + TimeDelta::hours(4))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let to_run = receiving.id.to_string();
    let stop = format!("delivery_{}", delivery.id);
    let arrival = format!("arrival_{}", delivery.id);
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/hand-over", run.id),
        &[
            ("to_run_id", to_run.as_str()),
            ("reason", "breakdown"),
            (stop.as_str(), "on"),
            (arrival.as_str(), late.as_str()),
        ],
    )
    .await
    .assert_status(422);
    assert!(HandOver::of_run(w.db(), run.id).await.unwrap().is_empty());
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Dispatched);

    w.abort(&run, "breakdown").await.assert_ok();

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::DeliveryFailed);
    assert_eq!(after.failure_reason.as_deref(), Some(reason::RUN_ABORTED));
    assert_eq!(after.delivery_reason.as_deref(), Some("run_aborted"));
    assert_eq!(after.failure_side.as_deref(), Some(side::OUTLET));
    assert_eq!(after.responsible_party.as_deref(), Some(party::BUSINESS));
    // Cooked food after an abort is discarded, not brought back.
    assert_eq!(
        w.delivery(&delivery).await.resolution.as_deref(),
        Some("discard")
    );
}

/// AC4: given the same stop handed over to a run arriving within the promise,
/// ORD-503 stays `dispatched`, and now follows the receiving rider's run.
#[renox::test]
async fn a_stop_handed_over_in_time_stays_dispatched_when_the_run_is_aborted() {
    let w = boot_own_riders().await;
    let (order, delivery, run) = prepaid(&w).await;
    let receiving = w.run_of(&w.other_rider).await;
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/dispatch", receiving.id),
        &[],
    )
    .await
    .assert_ok();
    let placed = w.delivery(&delivery).await.placed_at.expect("a placed time");
    let in_time = (placed + TimeDelta::minutes(35) + TimeDelta::hours(4))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let to_run = receiving.id.to_string();
    let stop = format!("delivery_{}", delivery.id);
    let arrival = format!("arrival_{}", delivery.id);
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/hand-over", run.id),
        &[
            ("to_run_id", to_run.as_str()),
            ("reason", "breakdown"),
            (stop.as_str(), "on"),
            (arrival.as_str(), in_time.as_str()),
        ],
    )
    .await
    .assert_ok();
    let proposal = HandOver::of_run(w.db(), run.id).await.unwrap().remove(0);
    let parcel = format!("parcel_{}", delivery.id);
    w.post(
        &w.other_rider,
        &format!("/driver/hand-overs/{}/confirm", proposal.id),
        &[(parcel.as_str(), "on")],
    )
    .await
    .assert_status(303);

    w.abort(&run, "breakdown").await.assert_ok();

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Dispatched);
    assert_eq!(after.failure_reason, None);
    assert_eq!(after.run_id, Some(receiving.id));
    assert_eq!(after.rider_user_id, Some(w.other_rider.id));
}

/// AC5: a second identical event leaves one history row.
#[renox::test]
async fn a_second_identical_event_changes_nothing_and_leaves_one_history_row() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = prepaid(&w).await;
    w.fail_stop(&delivery, "not_home").await.assert_status(303);

    // The same attempt is announced again.
    w.state()
        .emit(DeliveryAttemptFailed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: "not_home".to_owned(),
            attempt: 1,
        })
        .await
        .unwrap();

    let history = state_machine::history(w.db(), fnb::FnbOrder::TABLE, order.id)
        .await
        .unwrap();
    let failures: Vec<_> = history
        .iter()
        .filter(|row| row.transition == "fail_delivery")
        .collect();
    assert_eq!(failures.len(), 1, "one history row for the failure");
    assert_eq!(failures[0].from_state.as_deref(), Some("dispatched"));
    assert_eq!(failures[0].to_state, "delivery_failed");
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::DeliveryFailed);
    // The customer was told once.
    assert_eq!(customer_titles(&w).iter().filter(|t| t.starts_with("We could not")).count(), 1);
}

/// An event that arrives after the order moved on (a late `DeliveryAttemptFailed`
/// for a completed order) is ignored: no state change.
#[renox::test]
async fn a_late_failure_of_a_completed_order_is_ignored() {
    let w = boot_own_riders().await;
    let customer = w.customer("Mariam Al Hosani").await;
    let (order, delivery, _run) = w.dispatched(customer, "online", &PLATE).await;
    w.deliver_stop(&delivery, "0").await.assert_status(303);
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Completed);

    w.state()
        .emit(DeliveryAttemptFailed {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: "not_home".to_owned(),
            attempt: 1,
        })
        .await
        .unwrap();

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    assert_eq!(after.failure_reason, None);
}

/// AC6: given *refused* with cause *late*, the order reason is refused (late),
/// outlet side, party business; with cause *other* (plain *refused*), customer
/// side. The same table maps the other reasons, and *rider issue* is the
/// rider's.
#[renox::test]
async fn the_cause_of_a_refusal_decides_the_side_and_each_reason_is_mapped() {
    let w = boot_own_riders().await;
    let table = [
        ("refused_late", reason::REFUSED_LATE, side::OUTLET, party::BUSINESS),
        ("refused_wrong_item", reason::REFUSED_WRONG_ITEM, side::OUTLET, party::BUSINESS),
        ("refused", reason::REFUSED, side::CUSTOMER, party::CUSTOMER),
        ("wrong_address", reason::WRONG_ADDRESS, side::CUSTOMER, party::CUSTOMER),
        ("access_denied", reason::CUSTOMER_UNREACHABLE, side::CUSTOMER, party::CUSTOMER),
        ("rider_issue", reason::RIDER_ISSUE, side::OUTLET, party::RIDER),
    ];
    for (code, expected, expected_side, expected_party) in table {
        let (order, delivery, _run) = prepaid(&w).await;
        w.fail_stop(&delivery, code).await.assert_status(303);
        let after = w.reload(&order).await;
        assert_eq!(after.status, FnbOrderStatus::DeliveryFailed, "{code}");
        assert_eq!(after.failure_reason.as_deref(), Some(expected), "{code}");
        assert_eq!(after.failure_side.as_deref(), Some(expected_side), "{code}");
        assert_eq!(after.responsible_party.as_deref(), Some(expected_party), "{code}");
        assert_eq!(after.delivery_reason.as_deref(), Some(code), "{code}");
    }
}

/// The rider picks *rider issue* and the two causes of *refused* in the driver
/// PWA, in the same list as wl-delivery's own reasons.
#[renox::test]
async fn the_driver_pwa_offers_rider_issue_and_the_causes_of_a_refusal() {
    let w = boot_own_riders().await;
    let (_order, delivery, _run) = prepaid(&w).await;

    w.get(&w.rider, &format!("/driver/stops/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("Rider issue")
        .assert_see("Refused: the order was late")
        .assert_see("Refused: wrong item")
        .assert_see("Not home");
}

/// The order page says why it failed, with the reason wl-delivery kept, and
/// offers no reattempt and no manual failure; the system alone runs
/// `fail_delivery`.
#[renox::test]
async fn the_order_page_shows_the_failure_and_no_user_can_fail_a_delivery_by_hand() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = prepaid(&w).await;

    // While it is out for delivery nobody is offered a manual failure.
    for user in [&w.manager, &w.dispatcher, &w.cashier] {
        let offered = w.offered(user, &order).await;
        assert!(!offered.iter().any(|name| name == "fail_delivery"), "{offered:?}");
    }
    let tried = w.try_run(&w.manager, &order, "fail_delivery").await;
    assert_ne!(tried, "ok", "a user cannot run the system's transition");
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Dispatched);

    w.fail_stop(&delivery, "wrong_address").await.assert_status(303);

    w.get(&w.manager, &format!("/orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Delivery failed")
        .assert_see("Wrong address")
        .assert_see("wrong_address")
        .assert_see("No reattempt")
        .assert_dont_see("Put the delivery back in the queue");
    let offered = w.offered(&w.manager, &w.reload(&order).await).await;
    assert!(!offered.iter().any(|name| name == "fail_delivery"), "{offered:?}");
}

/// The kitchen manager changes the party proposed as responsible with a note;
/// nobody else may, and a note is needed.
#[renox::test]
async fn the_kitchen_manager_changes_the_proposed_party_with_a_note() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = prepaid(&w).await;
    w.fail_stop(&delivery, "rider_issue").await.assert_status(303);
    let path = format!("/orders/{}/responsible", order.id);
    assert_eq!(
        w.reload(&order).await.responsible_party.as_deref(),
        Some(party::RIDER)
    );

    w.post(&w.manager, &path, &[("party", "business"), ("note", "")])
        .await
        .assert_status(422);
    w.post(&w.nobody, &path, &[("party", "business"), ("note", "Van was not serviced")])
        .await
        .assert_forbidden();
    w.post(
        &w.manager,
        &path,
        &[("party", "business"), ("note", "The van was not serviced")],
    )
    .await
    .assert_status(303);

    let after = w.reload(&order).await;
    assert_eq!(after.responsible_party.as_deref(), Some(party::BUSINESS));
    assert_eq!(after.responsible_note.as_deref(), Some("The van was not serviced"));
    assert_eq!(after.responsible_changed_by, Some(w.manager.id));
    // The reason itself is the rider's as the delivery recorded it.
    assert_eq!(after.failure_reason.as_deref(), Some(reason::RIDER_ISSUE));
}
