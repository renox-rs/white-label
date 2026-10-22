//! BL-DLV-07 (#163), `WRK.RUL.16`, `WRK.RUL.08`, `STK.ACL.02`, X-12, 06-N9-b:
//! after a failed attempt a perishable delivery is never reattempted; the
//! policy of its source type returns it to the origin or has the driver
//! discard it with a photo and a reason, the wastage is recorded with the
//! source as reference, a wastage above the limit waits for a second person,
//! and a non-perishable delivery is still reattempted by `WRK.RUL.08`.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use renox::prelude::*;
use wl_delivery::{
    Deliveries, Delivery, DeliveryStatus, NewRun, PerishableAction, RunStatus, VehicleType,
    WlDelivery,
};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;
use wl_kernel::{DeliveryItem, DeliveryRequest, Subject};

use support::documents::Documents;
use support::{VAN, World, boot, boot_with};

static SOURCE: AtomicI64 = AtomicI64::new(7_000);

/// A perishable cooked-food delivery of an F&B order, with one line copied
/// from the order's line.
async fn food(w: &World, source_type: &str) -> Delivery {
    let source = SOURCE.fetch_add(1, Ordering::SeqCst);
    let request = DeliveryRequest::new(Subject::new(source_type, source), "Mariam Al Hosani")
        .address("Villa 12, Al Barsha 2", "Al Barsha")
        .phone("+971500000000")
        .item(
            DeliveryItem::new("Chicken meal", 2_000)
                .source_line(Subject::new("fnb.order_line", source * 10)),
        )
        .perishable();
    Deliveries::request(w.state(), request)
        .await
        .expect("the delivery is requested")
}

/// A dispatched run of the driver with `deliveries` on it.
async fn run_with(w: &World, deliveries: &[&Delivery]) -> wl_delivery::Run {
    w.valid_documents_for(w.driver.id, "2099-12-31");
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), w.driver.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap();
    for delivery in deliveries {
        w.assign(&w.dispatcher, delivery, &run).await.assert_ok();
    }
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    w.reload_run(&run).await
}

/// `delivery` on a run that left, then failed with `reason`.
async fn failed(w: &World, delivery: &Delivery, reason: &str) -> (Delivery, wl_delivery::Run) {
    let run = run_with(w, &[delivery]).await;
    w.fail(&w.driver, delivery, reason).await.assert_status(303);
    (w.reload(delivery).await, run)
}

/// The driver submits the "discard with photo" task of `delivery`.
async fn discard(w: &World, delivery: &Delivery) -> renox::testing::TestResponse {
    w.post(
        &w.driver,
        &format!("/driver/stops/{}/discard", delivery.id),
        &[
            ("reason", "Spoilt in the heat"),
            ("photo", "discard-photo-1"),
        ],
    )
    .await
}

async fn pending(w: &World, delivery: &Delivery) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
}

/// AC1: on the module default policy (*discard* for cooked food) a perishable
/// delivery that fails with *not home* is not reattempted and the driver gets
/// the "discard with photo" task on the stop screen.
#[renox::test]
async fn a_failed_perishable_delivery_is_not_reattempted_and_the_driver_gets_the_discard_task() {
    let w = boot().await;
    let delivery = food(&w, "fnb.order").await;

    let (after, _run) = failed(&w, &delivery, "not_home").await;

    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.resolution.as_deref(), Some("discard"));
    w.post(
        &w.manager,
        &format!("/deliveries/{}/reattempt", after.id),
        &[],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&after).await.status, DeliveryStatus::Attempted);
    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", after.id))
        .await
        .assert_ok()
        .assert_see("Discard with photo");
    w.app.acting_as(&w.manager);
    w.app
        .get(&format!("/deliveries/{}", after.id))
        .await
        .assert_ok()
        .assert_dont_see("Put the delivery back in the queue");
    assert!(
        w.notices
            .users_of_type("delivery.perishable_failed")
            .contains(&w.dispatcher.id)
    );
}

/// AC2: the driver submits a photo and a reason: the delivery is
/// `discarded`, one wastage record exists with the order as reference, the
/// dispatchers are told and `DeliveryDiscarded` is emitted.
#[renox::test]
async fn the_drivers_photo_and_reason_discard_the_goods_and_record_the_wastage() {
    let w = boot().await;
    w.wastage.worth(9_000);
    let delivery = food(&w, "fnb.order").await;
    failed(&w, &delivery, "not_home").await;

    discard(&w, &delivery).await.assert_status(303);

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Discarded);
    assert_eq!(after.discard_reason.as_deref(), Some("Spoilt in the heat"));
    assert_eq!(after.discard_photo_ref.as_deref(), Some("discard-photo-1"));
    let recorded = w.wastage.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].source, delivery.source());
    assert_eq!(recorded[0].reason, "Spoilt in the heat");
    assert_eq!(recorded[0].lines.len(), 1);
    let events = w.discarded.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, delivery.source());
    assert_eq!(events[0].value, 9_000);
    assert!(events[0].wastage.is_some());
    assert_eq!(after.wastage_value, 9_000);
    assert!(
        w.notices
            .users_of_type("delivery.discarded")
            .contains(&w.dispatcher.id)
    );
}

/// The discard needs a photo and a reason, and only the run's driver may
/// submit it; nothing is recorded meanwhile.
#[renox::test]
async fn a_discard_without_a_photo_or_a_reason_or_from_another_driver_is_refused() {
    let w = boot().await;
    let delivery = food(&w, "fnb.order").await;
    failed(&w, &delivery, "not_home").await;
    let path = format!("/driver/stops/{}/discard", delivery.id);

    w.post(&w.driver, &path, &[("reason", "Spoilt"), ("photo", "")])
        .await
        .assert_status(422);
    w.post(&w.driver, &path, &[("reason", " "), ("photo", "p-1")])
        .await
        .assert_status(422);
    w.post(
        &w.other_driver,
        &path,
        &[("reason", "Spoilt"), ("photo", "p-1")],
    )
    .await
    .assert_status(404);

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    assert!(w.wastage.recorded().is_empty());
    assert!(w.discarded.all().is_empty());
}

/// AC3: with a limit of AED 200 and a wastage of AED 250 the discard waits
/// for approval by someone other than the driver; the delivery stays
/// `attempted` and nothing is recorded until then.
#[renox::test]
async fn a_wastage_above_the_limit_waits_for_a_second_person() {
    let w = boot_with(
        WlDelivery::new().discard_approval_limit(20_000),
        Documents::logistics(),
    )
    .await;
    w.wastage.worth(25_000);
    let delivery = food(&w, "fnb.order").await;
    failed(&w, &delivery, "not_home").await;

    discard(&w, &delivery).await.assert_status(303);

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    assert!(w.wastage.recorded().is_empty());
    let requests = pending(&w, &delivery).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].requester_id, w.driver.id);
    // The driver cannot approve their own request.
    w.post(
        &w.driver,
        &format!("/approvals/{}/approve", requests[0].id),
        &[("comment", "ok")],
    )
    .await
    .assert_forbidden();
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);

    w.post(
        &w.stock_approver,
        &format!("/approvals/{}/approve", requests[0].id),
        &[("comment", "seen")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Discarded);
    assert_eq!(w.wastage.recorded().len(), 1);
    assert_eq!(w.discarded.all().len(), 1);
}

/// A wastage within the limit needs no approval.
#[renox::test]
async fn a_wastage_within_the_limit_is_discarded_at_once() {
    let w = boot_with(
        WlDelivery::new().discard_approval_limit(20_000),
        Documents::logistics(),
    )
    .await;
    w.wastage.worth(15_000);
    let delivery = food(&w, "fnb.order").await;
    failed(&w, &delivery, "not_home").await;

    discard(&w, &delivery).await.assert_status(303);

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Discarded);
    assert!(pending(&w, &delivery).await.is_empty());
}

/// AC4: with the approval rejected the delivery stays `attempted`; the
/// dispatcher can run `return`, which becomes `returned` once the food is
/// confirmed at the outlet.
#[renox::test]
async fn after_a_rejection_the_manager_returns_the_food_once_it_is_confirmed() {
    let w = boot_with(
        WlDelivery::new().discard_approval_limit(20_000),
        Documents::logistics(),
    )
    .await;
    w.wastage.worth(25_000);
    let delivery = food(&w, "fnb.order").await;
    failed(&w, &delivery, "not_home").await;
    discard(&w, &delivery).await.assert_status(303);
    let request = pending(&w, &delivery).await.remove(0);

    w.post(
        &w.stock_approver,
        &format!("/approvals/{}/reject", request.id),
        &[("comment", "Not that much was spoilt")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    let path = format!("/deliveries/{}/return", delivery.id);
    w.post(&w.manager, &path, &[("goods_confirmed", "0")])
        .await
        .assert_status(422);
    w.post(&w.driver, &path, &[("goods_confirmed", "1")])
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);

    w.post(&w.manager, &path, &[("goods_confirmed", "1")])
        .await
        .assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Returned);
    let returned = w.returned.all();
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].source, delivery.source());
    assert_eq!(returned[0].reason, "not_home");
    assert!(w.wastage.recorded().is_empty());
}

fn own_riders() -> WlDelivery {
    WlDelivery::new().perishable_policy(
        "fnb.order",
        PerishableAction::Return,
        PerishableAction::Discard,
    )
}

/// AC5: a 06 client (policy *return* for own-rider deliveries) and cooked
/// food that fails with the rider unable to deliver: no discard task is
/// created; the delivery becomes `returned` when the food is confirmed at
/// the outlet and `DeliveryReturned` is emitted.
#[renox::test]
async fn under_the_return_policy_there_is_no_discard_task_and_the_food_returns_when_confirmed() {
    let w = boot_with(own_riders(), Documents::logistics()).await;
    let delivery = food(&w, "fnb.order").await;
    let (after, run) = failed(&w, &delivery, "not_home").await;
    assert_eq!(after.resolution.as_deref(), Some("return"));
    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", after.id))
        .await
        .assert_ok()
        .assert_dont_see("Discard with photo");
    discard(&w, &after).await.assert_status(422);

    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    assert_eq!(w.reload(&after).await.status, DeliveryStatus::Attempted);
    let item = format!("item_{}_0", after.id);
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/receipts", run.id),
        &[(item.as_str(), "received")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload(&after).await.status, DeliveryStatus::Returned);
    assert_eq!(w.returned.all().len(), 1);
    assert!(w.wastage.recorded().is_empty());
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Closed);
}

/// AC6: a 06 client and a run aborted with a perishable stop the rider cannot
/// bring back: the stop gets the discard task at once and is never
/// reattempted.
#[renox::test]
async fn an_aborted_run_gives_its_perishable_stop_the_discard_task_at_once() {
    let w = boot_with(own_riders(), Documents::logistics()).await;
    let delivery = food(&w, "fnb.order").await;
    let run = run_with(&w, &[&delivery]).await;

    w.post(
        &w.dispatcher,
        &format!("/runs/{}/abort", run.id),
        &[("reason", "accident")],
    )
    .await
    .assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.resolution.as_deref(), Some("discard"));
    assert_eq!(after.attempts, 0, "run aborted does not count");
    w.post(
        &w.manager,
        &format!("/deliveries/{}/reattempt", after.id),
        &[],
    )
    .await
    .assert_status(422);
    discard(&w, &after).await.assert_status(303);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Discarded);
}

/// AC7: a non-perishable delivery with 1 of 3 attempts is still reattempted
/// by `WRK.RUL.08` (the rule is on for this client).
#[renox::test]
async fn a_non_perishable_delivery_with_attempts_left_is_reattempted_by_the_rule() {
    let w = boot_with(WlDelivery::new().attempt_rule(true), Documents::logistics()).await;
    let delivery = w.simple_delivery().await;

    let (after, _run) = failed(&w, &delivery, "not_home").await;

    assert_eq!(after.status, DeliveryStatus::Pending);
    assert_eq!(after.attempts, 1);
    assert_eq!(after.run_id, None);
    assert_eq!(after.resolution, None);
}

/// With no attempts left the rule decides *return*; the goods come back and
/// the system returns the delivery.
#[renox::test]
async fn a_non_perishable_delivery_without_attempts_left_is_returned_when_the_goods_are_back() {
    let w = boot_with(WlDelivery::new().attempt_rule(true), Documents::logistics()).await;
    let delivery = w.simple_delivery().await;
    // Only one attempt is allowed: the visit that fails is the last.
    Delivery::query()
        .where_(Delivery::ID.eq(delivery.id))
        .update(w.db(), &[("max_attempts", &1_i64)])
        .await
        .unwrap();
    let (after, run) = failed(&w, &delivery, "not_home").await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.attempts, 1);
    assert_eq!(after.resolution.as_deref(), Some("return"));

    w.post(&w.driver, &format!("/driver/runs/{}/return", run.id), &[])
        .await
        .assert_ok();
    let item = format!("item_{}_0", after.id);
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/receipts", run.id),
        &[(item.as_str(), "received")],
    )
    .await
    .assert_ok();

    assert_eq!(w.reload(&after).await.status, DeliveryStatus::Returned);
}

/// Without the rule the dispatcher decides: a manager reattempts a failed
/// non-perishable delivery by hand, and the driver cannot.
#[renox::test]
async fn a_manager_reattempts_a_failed_delivery_by_hand() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let (after, _run) = failed(&w, &delivery, "not_home").await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    let path = format!("/deliveries/{}/reattempt", after.id);

    w.post(&w.driver, &path, &[]).await.assert_forbidden();
    w.post(&w.manager, &path, &[]).await.assert_ok();

    let back = w.reload(&after).await;
    assert_eq!(back.status, DeliveryStatus::Pending);
    assert_eq!(back.run_id, None);
}

/// The rule acts once per failed attempt.
#[renox::test]
async fn the_rule_acts_once_per_failed_attempt() {
    let w = boot().await;
    let delivery = food(&w, "fnb.order").await;
    let (after, _run) = failed(&w, &delivery, "not_home").await;
    let told = w.notices.of_type("delivery.perishable_failed").len();

    wl_delivery::attempted::after_failed_attempt(w.state(), after.id, false)
        .await
        .unwrap();

    assert_eq!(w.notices.of_type("delivery.perishable_failed").len(), told);
    assert_eq!(
        w.reload(&after).await.resolution.as_deref(),
        Some("discard")
    );
}
