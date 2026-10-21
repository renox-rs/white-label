//! The base of BL-DLV-01 (#157) and BL-DLV-02 (#158), `WRK.DSP.01` and
//! `WRK.POD.01`: a delivery is requested, put on a run, the run is
//! dispatched, the driver sees the stop on the day list and records the
//! outcome. Every status change goes through the machines and leaves a
//! history row; the events the sources listen to are emitted.

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_delivery::{
    Attempt, AttemptOutcome, Delivery, DeliveryStatus, NewRun, Run, RunStatus, VehicleType,
};
use wl_kernel::delivery::requester;
use wl_kernel::{DeliveryItem, DeliveryRequest, Subject};

use support::{boot, stop_fields};

/// The delivery goes pending, assigned, out for delivery and delivered, with
/// the events of the spec and a history row for every change.
#[renox::test]
async fn a_delivery_goes_from_pending_to_delivered_with_its_events_and_history() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let delivery = w.simple_delivery().await;
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert!(delivery.number.starts_with("DLV-"));
    let run = w
        .run_on(&wl_kernel::documents::today().format("%Y-%m-%d").to_string())
        .await;
    assert!(run.number.starts_with("RUN-"));

    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();
    let assigned = w.reload(&delivery).await;
    assert_eq!(assigned.status, DeliveryStatus::Assigned);
    assert_eq!(assigned.run_id, Some(run.id));
    assert_eq!(assigned.run_sequence, Some(1));
    let events = w.assigned.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delivery, delivery.delivery_id());
    assert_eq!(events[0].driver, w.driver.id);

    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Dispatched);
    let out = w.reload(&delivery).await;
    assert_eq!(out.status, DeliveryStatus::OutForDelivery);
    assert_eq!(w.out.all().len(), 1);
    let dispatched = w.dispatched.all();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].deliveries, vec![delivery.delivery_id()]);
    assert_eq!(dispatched[0].vehicle, Some(support::VAN));

    let line = &w.lines(&out).await[0];
    let fields = stop_fields(&[(line.id, "10", "0", "", "")], "0");
    w.deliver(&w.driver, &out, &fields).await;
    let done = w.reload(&delivery).await;
    assert_eq!(done.status, DeliveryStatus::Delivered);
    assert_eq!(done.attempts, 1);
    let attempts = done.attempts_made(w.db()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Delivered);
    assert_eq!(attempts[0].actor_user_id, Some(w.driver.id));
    assert!(attempts[0].latitude_micro.is_some());
    assert_eq!(w.delivered.all().len(), 1);

    let history = state_machine::history(w.db(), Delivery::TABLE, delivery.id)
        .await
        .unwrap();
    let names: Vec<_> = history.iter().map(|row| row.transition.as_str()).collect();
    assert_eq!(names, ["assign", "dispatch", "deliver"]);
    let run_history = state_machine::history(w.db(), Run::TABLE, run.id)
        .await
        .unwrap();
    assert_eq!(run_history.len(), 1);
    assert_eq!(run_history[0].transition, "dispatch");
}

/// `unassign` puts an assigned delivery back to pending while the run has
/// not left, and a dispatched run's delivery cannot be taken off.
#[renox::test]
async fn unassign_returns_the_delivery_to_pending_until_the_run_leaves() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let delivery = w.simple_delivery().await;
    let run = w.run_on("2026-12-01").await;
    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    w.app.acting_as(&w.dispatcher);
    w.app
        .htmx()
        .post(&format!("/deliveries/{}/unassign", delivery.id), &[])
        .await
        .assert_ok();
    let back = w.reload(&delivery).await;
    assert_eq!(back.status, DeliveryStatus::Pending);
    assert_eq!(back.run_id, None);

    let out = w.out_for_delivery(&delivery).await;
    w.app.acting_as(&w.dispatcher);
    w.app
        .htmx()
        .post(&format!("/deliveries/{}/unassign", out.id), &[])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
}

/// A run is dispatched only from `planned`, and only by `runs.dispatch`.
#[renox::test]
async fn only_a_dispatcher_dispatches_a_planned_run_once() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let run = w.run_on("2026-12-01").await;
    w.dispatch(&w.driver, &run).await.assert_forbidden();
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Planned);
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    w.dispatch(&w.dispatcher, &run).await.assert_status(422);
    assert_eq!(w.dispatched.all().len(), 1);
}

/// The driver sees only the stops of their own runs for the day, and the
/// recipient's phone and full address only while the stop is open
/// (`WRK.ACL.05`).
#[renox::test]
async fn the_driver_sees_only_their_own_stops_and_the_phone_only_while_open() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;

    w.app.acting_as(&w.driver);
    w.app
        .get("/driver")
        .await
        .assert_ok()
        .assert_see("Mariam Al Hosani")
        .assert_see("Villa 12, Al Barsha 2");
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_ok()
        .assert_see("+971500000000");

    w.app.acting_as(&w.other_driver);
    w.app
        .get("/driver")
        .await
        .assert_ok()
        .assert_dont_see("Mariam Al Hosani");
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_not_found();

    let line = &w.lines(&out).await[0];
    w.deliver(
        &w.driver,
        &out,
        &stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await;
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_ok()
        .assert_dont_see("+971500000000")
        .assert_dont_see("Villa 12");
}

/// Only the driver of the run records its outcomes, and only with
/// `deliveries.complete`.
#[renox::test]
async fn only_the_driver_of_the_run_records_the_outcome() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    let line = &w.lines(&out).await[0];
    let fields = stop_fields(&[(line.id, "10", "0", "", "")], "0");

    w.deliver(&w.other_driver, &out, &fields)
        .await
        .assert_status(422)
        .assert_see("Only the driver of the run");
    w.deliver(&w.dispatcher, &out, &fields)
        .await
        .assert_forbidden();
    w.fail(&w.other_driver, &out, "not_home")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    assert!(w.delivered.all().is_empty());
}

/// A failed visit saves the attempt with the reason and the location, counts
/// against the maximum unless the reason says it does not, and emits
/// `DeliveryAttemptFailed`.
#[renox::test]
async fn a_failed_visit_is_saved_and_counted_unless_the_reason_does_not_count() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;

    w.app.acting_as(&w.driver);
    w.app
        .htmx()
        .post(
            &format!("/driver/stops/{}/fail", out.id),
            &[("reason", "not_home")],
        )
        .await
        .assert_status(422)
        .assert_see("location");

    w.fail(&w.driver, &out, "not_home").await;
    let after = w.reload(&out).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.attempts, 1);
    let attempts = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(out.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Failed);
    assert_eq!(attempts[0].failure_reason.as_deref(), Some("not_home"));
    assert!(attempts[0].counts);
    let events = w.failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "not_home");
    assert_eq!(events[0].attempt, 1);
}

/// A reason that does not count (*not attempted*) leaves the maximum alone;
/// an unknown reason is refused.
#[renox::test]
async fn a_reason_that_does_not_count_leaves_the_attempts_alone_and_an_unknown_one_is_refused() {
    let w = boot().await;
    let delivery = w.simple_delivery().await;
    let out = w.out_for_delivery(&delivery).await;
    w.fail(&w.driver, &out, "no_such_reason")
        .await
        .assert_status(422);
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);
    w.fail(&w.driver, &out, "not_attempted").await;
    let after = w.reload(&out).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    assert_eq!(after.attempts, 0);
}

/// A source asks for a delivery through the kernel's requester and takes it
/// back; a request without items is refused.
#[renox::test]
async fn a_source_requests_and_cancels_a_delivery_through_the_kernel() {
    let w = boot().await;
    let requester = requester(w.state());
    let none = DeliveryRequest::new(Subject::new("sales.order", 900), "Mariam");
    assert!(requester.request(w.state(), none).await.is_err());

    let request = DeliveryRequest::new(Subject::new("sales.order", 901), "Mariam")
        .item(DeliveryItem::new("Tiles", 4_000))
        .perishable();
    let id = requester.request(w.state(), request).await.unwrap();
    let delivery = w.delivery(id.0).await;
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert_eq!(delivery.max_attempts, 1);
    assert_eq!(delivery.pod_methods, "photo,signature");
    assert!(delivery.partial_acceptance);

    requester
        .cancel(w.state(), id, "the order was cancelled")
        .await
        .unwrap();
    let cancelled = w.reload(&delivery).await;
    assert_eq!(cancelled.status, DeliveryStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("the order was cancelled")
    );
}

/// A van needs its vehicle and an on-foot rider has none.
#[renox::test]
async fn a_run_needs_its_vehicle_unless_the_rider_is_on_foot() {
    let w = boot().await;
    let day = support::date("2026-12-01");
    let state = w.state();
    assert!(
        wl_delivery::plan::create_run(state, NewRun::new(day, w.driver.id))
            .await
            .is_err()
    );
    assert!(
        wl_delivery::plan::create_run(
            state,
            NewRun::new(day, w.driver.id).vehicle(VehicleType::OnFoot, Some(3))
        )
        .await
        .is_err()
    );
    let rider = w.on_foot_run_on("2026-12-01").await;
    assert_eq!(rider.vehicle_asset_id, None);
    assert_eq!(rider.status, RunStatus::Planned);
}
