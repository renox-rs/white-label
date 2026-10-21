//! BL-DLV-08 (#164), `X-12`: a run is aborted mid-route. The stops still out
//! fail with *run aborted* (not counted), recipients and dispatchers are told,
//! an optional vehicle ticket is opened, the driver's cash handover stays to
//! be counted at the depot, and `RunAborted` lists the stops that failed.

mod support;

use renox::prelude::*;
use wl_delivery::delivery::Attempt;
use wl_delivery::hand_over::HandOver;
use wl_delivery::{Delivery, DeliveryStatus, NewRun, Run, RunStatus, VehicleType, WlDelivery};
use wl_kernel::custody::HandoverStatus;
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyRecipient;

use support::{VAN, World, boot_custody_with, boot_with_custody, stop_fields};

async fn dispatched_run(w: &World, driver: &User, deliveries: &[&Delivery]) -> Run {
    w.valid_documents_for(driver.id, "2099-12-31");
    let run = wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), driver.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap();
    for delivery in deliveries {
        w.assign(&w.dispatcher, delivery, &run).await.assert_ok();
    }
    w.dispatch(&w.dispatcher, &run).await.assert_ok();
    w.reload_run(&run).await
}

/// `count` new deliveries of one tile line.
async fn simple(w: &World, count: usize) -> Vec<Delivery> {
    let mut all = Vec::new();
    for _ in 0..count {
        all.push(w.simple_delivery().await);
    }
    all
}

async fn deliver_cash(w: &World, delivery: &Delivery, cash: &str) {
    let line = w.lines(delivery).await.remove(0);
    w.deliver(
        &w.driver,
        delivery,
        &stop_fields(&[(line.id, "10", "0", "", "")], cash),
    )
    .await
    .assert_status(303);
}

/// The dispatcher aborts `run`.
async fn abort(
    w: &World,
    run: &Run,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    w.post(&w.dispatcher, &format!("/runs/{}/abort", run.id), fields)
        .await
}

/// Hands `stops` of `run` over to `to_run` (proposal and confirmation).
async fn hand_over(w: &World, run: &Run, to_run: &Run, stops: &[&Delivery]) {
    let mut owned = vec![
        ("to_run_id".to_owned(), to_run.id.to_string()),
        ("reason".to_owned(), "breakdown".to_owned()),
    ];
    for stop in stops {
        owned.push((format!("delivery_{}", stop.id), "on".to_owned()));
    }
    let fields: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(&w.dispatcher, &format!("/runs/{}/hand-over", run.id), &fields)
        .await
        .assert_ok();
    let record = HandOver::of_run(w.db(), run.id)
        .await
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    let mut parcels: Vec<(String, String)> = stops
        .iter()
        .map(|d| (format!("parcel_{}", d.id), "on".to_owned()))
        .collect();
    parcels.push(("cash".to_owned(), "0".to_owned()));
    let fields: Vec<(&str, &str)> = parcels
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(
        &w.other_driver,
        &format!("/driver/hand-overs/{}/confirm", record.id),
        &fields,
    )
    .await
    .assert_status(303);
}

/// AC1: a `dispatched` run with 6 open stops, of which 2 were handed over,
/// aborted by the dispatcher with *breakdown* and the ticket ticked: the 4
/// stops left are `attempted` with *run aborted*, their attempt counts are
/// unchanged, the 4 recipients are messaged, a breakdown ticket exists for
/// the vehicle and `RunAborted` lists the 4 deliveries. The run is not closed
/// here.
#[renox::test]
async fn aborting_with_a_breakdown_fails_the_four_stops_left_and_opens_the_ticket() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 6).await;
    let refs: Vec<&Delivery> = stops.iter().collect();
    let r1 = dispatched_run(&w, &w.driver, &refs).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    hand_over(&w, &r1, &r2, &[&stops[0], &stops[1]]).await;

    abort(
        &w,
        &r1,
        &[("reason", "breakdown"), ("ticket", "breakdown")],
    )
    .await
    .assert_ok();

    for stop in &stops[2..] {
        let failed = w.reload(stop).await;
        assert_eq!(failed.status, DeliveryStatus::Attempted);
        assert_eq!(failed.attempts, 0, "run aborted does not count");
        let attempts = Attempt::query()
            .where_(Attempt::DELIVERY_ID.eq(stop.id))
            .get(w.db())
            .await
            .unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].failure_reason.as_deref(), Some("run_aborted"));
        assert!(!attempts[0].counts);
        assert_eq!(attempts[0].run_id, Some(r1.id));
    }
    for stop in &stops[..2] {
        let handed = w.reload(stop).await;
        assert_eq!(handed.status, DeliveryStatus::OutForDelivery);
        assert_eq!(handed.run_id, Some(r2.id));
    }
    // The recipients are messaged about the delay.
    let delayed = w.notices.of_type("delivery.run_delayed");
    assert_eq!(delayed.len(), 4);
    for request in &delayed {
        assert!(matches!(
            request.recipients[0],
            NotifyRecipient::Address { .. }
        ));
    }
    // A breakdown ticket exists for the vehicle.
    let tickets = w.tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, w.vehicle_subject());
    assert!(tickets[0].title.contains("breakdown"));
    assert_eq!(tickets[0].source, Some(r1.subject()));
    // `RunAborted` lists the 4 deliveries.
    let events = w.aborted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].run, r1.run_id());
    assert_eq!(events[0].driver, w.driver.id);
    assert_eq!(events[0].reason, "breakdown");
    assert_eq!(events[0].deliveries.len(), 4);
    assert_eq!(events[0].aborted_by, w.dispatcher.id);
    // The failed attempts are announced.
    let failed = w.failed.all();
    assert_eq!(failed.len(), 4);
    assert!(failed.iter().all(|e| e.reason == "run_aborted"));
    // The dispatchers are told.
    assert!(
        w.notices
            .users_of_type("delivery.run_aborted")
            .contains(&w.dispatcher.id)
    );
    // The run is back, but `close` stays a separate step: the failed stops
    // still have to be received at the depot.
    let back = w.reload_run(&r1).await;
    assert_eq!(back.status, RunStatus::ReturnedToDepot);
    assert_eq!(back.abort_reason.as_deref(), Some("breakdown"));
    assert_eq!(back.aborted_by, Some(w.dispatcher.id));
    assert!(back.aborted_at.is_some());
}

/// Without the tick no ticket is opened.
#[renox::test]
async fn no_ticket_is_opened_unless_it_is_ticked() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 1).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0]]).await;

    abort(&w, &run, &[("reason", "weather")]).await.assert_ok();

    assert!(w.tickets.opened().is_empty());
    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);
}

/// AC2: the run is `returned_to_depot` and a cash handover is open when AED
/// 400 was collected: the driver's period ends in the custody and the handover
/// of the run expects AED 400, waiting to be counted. `RunAborted` carries
/// the cash.
#[renox::test]
async fn an_aborted_run_leaves_the_cash_handover_open() {
    let w = boot_with_custody().await;
    let cash = w.request(&[("Tiles", 10_000, None)], 40_000).await;
    let open = w.simple_delivery().await;
    let run = dispatched_run(&w, &w.driver, &[&cash, &open]).await;
    deliver_cash(&w, &cash, "400.00").await;

    abort(&w, &run, &[("reason", "accident")]).await.assert_ok();

    let back = w.reload_run(&run).await;
    assert_eq!(back.status, RunStatus::ReturnedToDepot);
    let ended = w.custody.ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].0, w.driver.id);
    assert_eq!(ended[0].1, run.subject());
    let handover = wl_delivery::close::handover_of(w.state(), &back)
        .await
        .unwrap()
        .expect("the driver's handover of the run");
    assert_eq!(handover.status, HandoverStatus::Open);
    assert_eq!(handover.expected, 40_000);
    assert_eq!(w.aborted.all()[0].cash_collected, 40_000);
    // It is counted at the depot as usual; the run closes once the stop that
    // failed is received back as well.
    w.custody
        .count(w.state(), w.driver.id, &run.subject(), 40_000, w.dispatcher.id)
        .await;
    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);
}

/// No cash collected: no cash handover is opened.
#[renox::test]
async fn no_cash_handover_is_opened_when_nothing_was_collected() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 2).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0], &stops[1]]).await;

    abort(&w, &run, &[("reason", "driver_unwell")])
        .await
        .assert_ok();

    let back = w.reload_run(&run).await;
    assert!(
        wl_delivery::close::handover_of(w.state(), &back)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(w.aborted.all()[0].cash_collected, 0);
}

/// AC3: the driver can abort only with a reason: without one the phone's
/// abort is refused and the run stays out; with one it is aborted and the
/// driver goes back to the day list (303). Another driver cannot abort it.
#[renox::test]
async fn the_driver_aborts_from_the_phone_only_with_a_reason() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 2).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0], &stops[1]]).await;
    let path = format!("/driver/runs/{}/abort", run.id);

    w.post(&w.driver, &path, &[("reason", "")])
        .await
        .assert_status(422)
        .assert_see("Choose why");
    w.post(&w.driver, &path, &[("reason", "boredom")])
        .await
        .assert_status(422);
    w.post(&w.other_driver, &path, &[("reason", "weather")])
        .await
        .assert_status(422)
        .assert_see("Only the driver");
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Dispatched);
    assert!(w.aborted.all().is_empty());

    w.post(&w.driver, &path, &[("reason", "weather")])
        .await
        .assert_status(303);

    assert_eq!(w.reload_run(&run).await.status, RunStatus::ReturnedToDepot);
    assert_eq!(w.aborted.all()[0].aborted_by, w.driver.id);
    assert_eq!(
        w.reload(&stops[0]).await.status,
        DeliveryStatus::Attempted
    );
}

/// Edge: the reason *other* needs a note.
#[renox::test]
async fn the_reason_other_needs_a_note() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 1).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0]]).await;

    abort(&w, &run, &[("reason", "other")])
        .await
        .assert_status(422)
        .assert_see("needs a note");
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Dispatched);
    assert_eq!(
        w.reload(&stops[0]).await.status,
        DeliveryStatus::OutForDelivery
    );

    abort(&w, &run, &[("reason", "other"), ("note", "Road closed")])
        .await
        .assert_ok();
    let back = w.reload_run(&run).await;
    assert_eq!(back.abort_note.as_deref(), Some("Road closed"));
}

/// Edge: a run that is already back at the depot cannot be aborted, and
/// neither can a planned one; a driver cannot abort through the dispatcher's
/// route.
#[renox::test]
async fn only_a_dispatched_run_is_aborted_and_by_the_right_role() {
    let w = boot_with_custody().await;
    let stops = simple(&w, 1).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0]]).await;
    let planned = {
        w.valid_documents_for(w.other_driver.id, "2099-12-31");
        wl_delivery::plan::create_run(
            w.state(),
            NewRun::new(today(), w.other_driver.id).vehicle(VehicleType::Van, Some(VAN)),
        )
        .await
        .unwrap()
    };

    w.post(
        &w.driver,
        &format!("/runs/{}/abort", run.id),
        &[("reason", "weather")],
    )
    .await
    .assert_status(403);
    abort(&w, &planned, &[("reason", "weather")])
        .await
        .assert_status(422);
    abort(&w, &run, &[("reason", "weather")]).await.assert_ok();
    abort(&w, &run, &[("reason", "weather")])
        .await
        .assert_status(422);
    assert_eq!(w.aborted.all().len(), 1);
}

/// 03: the abort reasons list adds *driver unreachable* (`LOG.RUL.13`); the
/// default list does not have it.
#[renox::test]
async fn a_vertical_adds_driver_unreachable_to_the_reasons() {
    let plain = boot_with_custody().await;
    let stops = simple(&plain, 1).await;
    let run = dispatched_run(&plain, &plain.driver, &[&stops[0]]).await;
    abort(&plain, &run, &[("reason", "driver_unreachable")])
        .await
        .assert_status(422);

    let w = boot_custody_with(WlDelivery::new().abort_reasons(&["driver_unreachable"])).await;
    let stops = simple(&w, 1).await;
    let run = dispatched_run(&w, &w.driver, &[&stops[0]]).await;
    abort(&w, &run, &[("reason", "driver_unreachable")])
        .await
        .assert_ok();
    assert_eq!(
        w.reload_run(&run).await.abort_reason.as_deref(),
        Some("driver_unreachable")
    );
}
