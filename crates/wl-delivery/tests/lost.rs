//! BL-DLV-06 (#162), `WRK.RUL.17`, `WRK.ACL.10`, `X-11`: a delivery whose
//! goods are neither delivered nor received back is lost: by the system after
//! 7 days untraced, or by a manager with a second manager's approval; a lost
//! delivery that turns up at a depot is found again. The ticket desk and the
//! events are recorded; approvals run through the kernel.

mod support;

use renox::db::DateTime;
use renox::prelude::*;
use wl_delivery::lost::{self, FOUND_REASON};
use wl_delivery::{Attempt, Delivery, DeliveryStatus, NewRun, Trace, VehicleType};
use wl_kernel::approvals::{self, ApprovalEnd, ApprovalRequest, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::tickets::TicketId;

use support::{VAN, World, boot};

/// `2026-11-01T06:00:00Z` (10:00 in Dubai) as an instant.
fn at(text: &str) -> DateTime {
    text.parse().expect("an instant")
}

/// A delivery that went out and whose visit failed: `attempted`.
async fn attempted(w: &World) -> Delivery {
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    w.fail(&w.driver, &out, "not_home").await.assert_status(303);
    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    after
}

/// Moves every moment the goods were traced (the dispatch, the visit, the
/// creation) to `when`, as if nothing had happened since.
async fn touched_at(w: &World, delivery: &Delivery, when: DateTime) {
    Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .update(w.db(), &[("occurred_at", &when)])
        .await
        .unwrap();
    Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .update(w.db(), &[("occurred_at", &when)])
        .await
        .unwrap();
    let created = Some(when);
    Delivery::query()
        .where_(Delivery::ID.eq(delivery.id))
        .update(w.db(), &[("created_at", &created)])
        .await
        .unwrap();
}

/// A scan of `delivery` recorded at `when`.
async fn scanned_at(w: &World, delivery: &Delivery, when: DateTime) {
    let mut scan = Trace {
        delivery_id: delivery.id,
        kind: "scan".into(),
        depot: "Sharjah depot".into(),
        occurred_at: when,
        ..Trace::default()
    };
    scan.insert(w.db()).await.unwrap();
}

async fn ask_loss(
    w: &World,
    by: &renox::prelude::User,
    delivery: &Delivery,
    liability: &str,
) -> renox::testing::TestResponse {
    w.post(
        by,
        &format!("/deliveries/{}/mark-lost", delivery.id),
        &[("reason", "Parcel untraced"), ("liability", liability)],
    )
    .await
}

async fn pending_request(w: &World, delivery: &Delivery) -> ApprovalRequest {
    ApprovalRequest::pending_about(w.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits for approval")
}

/// AC1: an `attempted` delivery last touched 2026-11-01 10:00; the rule runs
/// on 2026-11-08 02:00 (less than 7 x 24 h): nothing happens; on 2026-11-09
/// 02:00 it is `lost`, a claim ticket opens and `DeliveryLost` is emitted.
#[renox::test]
async fn the_rule_marks_a_delivery_lost_only_after_seven_full_days_untraced() {
    let w = boot().await;
    let delivery = attempted(&w).await;
    touched_at(&w, &delivery, at("2026-11-01T06:00:00Z")).await;

    // 2026-11-08 02:00 in Dubai.
    assert_eq!(
        lost::run(w.state(), at("2026-11-07T22:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    assert!(w.lost.all().is_empty());

    // 2026-11-09 02:00 in Dubai.
    assert_eq!(
        lost::run(w.state(), at("2026-11-08T22:00:00Z"))
            .await
            .unwrap(),
        1
    );

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Lost);
    assert_eq!(after.lost_liability.as_deref(), Some("none"));
    assert!(after.lost_reason.as_deref().unwrap().contains("7 days"));
    assert_eq!(after.claim_ticket_id, Some(1));
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].category_key, "delivery_claim");
    assert!(opened[0].title.contains(&delivery.number));
    let events = w.lost.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delivery, delivery.delivery_id());
    assert_eq!(events[0].source, delivery.source());
    assert!(!events[0].was_under_review);
    assert_eq!(events[0].liability, "none");
    assert_eq!(events[0].run, Some(wl_kernel::RunId(after.run_id.unwrap())));
    // A second run the same day picks nothing: the delivery is lost already.
    assert_eq!(
        lost::run(w.state(), at("2026-11-08T22:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(w.lost.all().len(), 1);
    assert_eq!(w.tickets.opened().len(), 1, "one claim ticket per loss");
}

/// AC2: a scan on 2026-11-05 restarts the clock from that time.
#[renox::test]
async fn a_scan_restarts_the_untraced_clock() {
    let w = boot().await;
    let delivery = attempted(&w).await;
    touched_at(&w, &delivery, at("2026-11-01T06:00:00Z")).await;
    scanned_at(&w, &delivery, at("2026-11-05T06:00:00Z")).await;

    // 2026-11-09 02:00 would be 7 days after the visit, but only 3 after the scan.
    assert_eq!(
        lost::run(w.state(), at("2026-11-08T22:00:00Z"))
            .await
            .unwrap(),
        0
    );
    // 2026-11-12 02:00 in Dubai: 6 days 16 hours after the scan.
    assert_eq!(
        lost::run(w.state(), at("2026-11-11T22:00:00Z"))
            .await
            .unwrap(),
        0
    );
    // 2026-11-13 02:00 in Dubai: more than 7 x 24 h after the scan.
    assert_eq!(
        lost::run(w.state(), at("2026-11-12T22:00:00Z"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Lost);
}

/// A scan recorded on the delivery page (`deliveries.manage`) is a trace as
/// well; a user without the permission cannot record one.
#[renox::test]
async fn a_scan_on_the_delivery_page_is_a_trace() {
    let w = boot().await;
    let delivery = attempted(&w).await;
    let before = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .count(w.db())
        .await
        .unwrap();

    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/trace", delivery.id),
        &[("kind", "scan"), ("depot", "Sharjah depot")],
    )
    .await
    .assert_ok();
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/trace", delivery.id),
        &[("kind", "scan"), ("depot", "Sharjah depot")],
    )
    .await
    .assert_forbidden();

    let after = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .count(w.db())
        .await
        .unwrap();
    assert_eq!(after, before + 1);
}

/// AC3: manager A requests `mark_lost` with a reason and liability *driver*;
/// the delivery stays `attempted` until manager B approves; then it is
/// `lost`, with the liability, a claim ticket and `DeliveryLost`. A cannot
/// approve their own request.
#[renox::test]
async fn a_manual_loss_needs_a_second_manager_to_approve() {
    let w = boot().await;
    let delivery = attempted(&w).await;

    ask_loss(&w, &w.ops_a, &delivery, "driver")
        .await
        .assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    let request = pending_request(&w, &delivery).await;
    assert_eq!(request.action, "delivery.mark_lost");
    assert_eq!(request.requester_id, w.ops_a.id);
    assert!(w.lost.all().is_empty());
    assert!(w.tickets.opened().is_empty());
    assert!(
        approvals::approve(w.state(), request.id, &w.ops_a, "mine")
            .await
            .is_err(),
        "the requester cannot approve"
    );
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);

    let decided = approvals::approve(w.state(), request.id, &w.ops_b, "agreed")
        .await
        .unwrap()
        .expect("the last step decides");
    assert_eq!(decided.decision, ApprovalEnd::Approved);

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Lost);
    assert_eq!(after.lost_liability.as_deref(), Some("driver"));
    assert_eq!(after.lost_reason.as_deref(), Some("Parcel untraced"));
    assert_eq!(after.claim_ticket_id, Some(1));
    let events = w.lost.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].liability, "driver");
    assert_eq!(w.tickets.opened().len(), 1);
}

/// AC3 (denied): the run's driver cannot request a loss, even holding
/// `deliveries.reverse`; one without it is refused at the route; the
/// liability must be one of the four; a delivered delivery is never lost
/// directly.
#[renox::test]
async fn the_runs_driver_cannot_request_and_the_request_is_checked() {
    let w = boot().await;
    // A chief driver holds `deliveries.reverse` besides driving.
    let chief = support::chief_driver(&w).await;
    w.valid_documents_for(chief.id, "2099-12-31");
    let delivery = w.simple_delivery().await;
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), chief.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap();
    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    let out = w.reload(&delivery).await;
    assert_eq!(out.status, DeliveryStatus::OutForDelivery);

    ask_loss(&w, &chief, &out, "none").await.assert_status(422);
    ask_loss(&w, &w.driver, &out, "none")
        .await
        .assert_forbidden();
    ask_loss(&w, &w.ops_a, &out, "carrier")
        .await
        .assert_status(422);
    assert!(
        ApprovalRequest::pending_about(w.db(), &out.delivery_id().subject())
            .await
            .unwrap()
            .is_empty()
    );

    // The same request from a manager who is not the driver is accepted.
    ask_loss(&w, &w.ops_a, &out, "business").await.assert_ok();
    assert_eq!(pending_request(&w, &out).await.requester_id, w.ops_a.id);
}

/// AC4: A's request is pending and the parcel is then scanned at the depot:
/// B's approval makes the request `failed`; the delivery stays `attempted`
/// and no claim ticket opens.
#[renox::test]
async fn a_scan_after_the_request_fails_it_at_final_approval() {
    let w = boot().await;
    let delivery = attempted(&w).await;
    ask_loss(&w, &w.ops_a, &delivery, "driver")
        .await
        .assert_ok();
    let request = pending_request(&w, &delivery).await;

    w.post(
        &w.ops_b,
        &format!("/deliveries/{}/trace", delivery.id),
        &[("kind", "scan"), ("depot", "Sharjah depot")],
    )
    .await
    .assert_ok();
    let decided = approvals::approve(w.state(), request.id, &w.ops_b, "agreed")
        .await
        .unwrap()
        .expect("the request ends");

    assert_eq!(decided.decision, ApprovalEnd::Failed);
    let stored = ApprovalRequest::find(w.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Failed);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Attempted);
    assert!(w.tickets.opened().is_empty());
    assert!(w.lost.all().is_empty());
}

/// AC5: a `lost` delivery confirmed at a depot: `found` makes it `attempted`
/// with a non-counting *found* visit, the claim ticket gets a comment (it is
/// not closed) and `DeliveryFound` is emitted. A depot is needed.
#[renox::test]
async fn a_found_delivery_is_attempted_again_with_a_visit_that_does_not_count() {
    let w = boot().await;
    let delivery = attempted(&w).await;
    ask_loss(&w, &w.ops_a, &delivery, "none").await.assert_ok();
    let request = pending_request(&w, &delivery).await;
    approvals::approve(w.state(), request.id, &w.ops_b, "agreed")
        .await
        .unwrap();
    let lost_delivery = w.reload(&delivery).await;
    assert_eq!(lost_delivery.status, DeliveryStatus::Lost);
    let counted = lost_delivery.attempts;

    // Only a holder of `deliveries.manage` runs it, and the depot is needed.
    w.post(
        &w.driver,
        &format!("/deliveries/{}/found", delivery.id),
        &[("depot", "Sharjah depot")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/found", delivery.id),
        &[("depot", "  ")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Lost);

    w.post(
        &w.ops_a,
        &format!("/deliveries/{}/found", delivery.id),
        &[("depot", "Sharjah depot")],
    )
    .await
    .assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.attempts, counted, "the found visit does not count");
    let visits = after.attempts_made(w.db()).await.unwrap();
    let last = visits.last().unwrap();
    assert_eq!(last.failure_reason.as_deref(), Some(FOUND_REASON));
    assert!(!last.counts);
    let events = w.found.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].depot, "Sharjah depot");
    assert_eq!(events[0].claim_ticket, Some(TicketId(1)));
    let comments = w.tickets.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, TicketId(1));
    assert!(comments[0].1.text.contains("Sharjah depot"));
    assert!(
        w.tickets.resolved().is_empty(),
        "the claim ticket is not closed"
    );
}

/// AC6: a `delivered` delivery is never picked by the rule, however old, and
/// cannot be marked lost directly: it goes through a review first.
#[renox::test]
async fn a_delivered_delivery_is_never_lost() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    touched_at(&w, &delivery, at("2026-01-01T06:00:00Z")).await;

    assert_eq!(
        lost::run(w.state(), at("2030-01-01T00:00:00Z"))
            .await
            .unwrap(),
        0
    );
    ask_loss(&w, &w.ops_a, &delivery, "none")
        .await
        .assert_status(422);

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
    assert!(w.lost.all().is_empty());
}

/// Edge: a delivery under review is lost by the rule once the review is old;
/// `DeliveryLost` says it was under review, with the review ticket, and the
/// frozen flag stays set.
#[renox::test]
async fn a_delivery_lost_from_a_review_says_so_and_stays_frozen() {
    let w = boot().await;
    let delivery = w.delivered_cod(30_000).await;
    w.open_review(&w.ops_a, &delivery, "recipient denies")
        .await
        .assert_ok();
    let reviewed = w.reload(&delivery).await;
    assert_eq!(reviewed.status, DeliveryStatus::UnderReview);
    touched_at(&w, &reviewed, at("2026-11-01T06:00:00Z")).await;

    assert_eq!(
        lost::run(w.state(), at("2026-11-08T22:00:00Z"))
            .await
            .unwrap(),
        1
    );

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Lost);
    assert!(after.frozen);
    let events = w.lost.all();
    assert_eq!(events.len(), 1);
    assert!(events[0].was_under_review);
    assert_eq!(events[0].ticket, Some(TicketId(1)), "the review ticket");
    assert_eq!(after.claim_ticket_id, Some(2), "and its own claim ticket");
}

/// The setting: the days untraced before a loss are a builder method.
#[renox::test]
async fn the_untraced_days_are_a_setting() {
    let w = support::boot_with(
        wl_delivery::WlDelivery::new().untraced_days(2),
        support::documents::Documents::logistics(),
    )
    .await;
    let delivery = attempted(&w).await;
    touched_at(&w, &delivery, at("2026-11-01T06:00:00Z")).await;

    assert_eq!(
        lost::run(w.state(), at("2026-11-02T22:00:00Z"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        lost::run(w.state(), at("2026-11-03T06:00:00Z"))
            .await
            .unwrap(),
        1
    );
}
