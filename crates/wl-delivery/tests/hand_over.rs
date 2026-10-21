//! BL-DLV-09 (#165), `X-12`, `FIN.CSH.01`: open stops are handed over to
//! another driver's run. The dispatcher proposes, the receiving driver
//! confirms each parcel and counts the cash, then the stops move, stay
//! `out_for_delivery`, and `RunHandedOver` carries the counted cash for
//! `wl-payments` to move between the drivers' handovers (BL-PAY-20; the cash
//! movement itself is tested against the real payments module in
//! `hand_over_real.rs`).

mod support;

use renox::chrono::Days;
use renox::prelude::*;
use wl_delivery::hand_over::HandOver;
use wl_delivery::{Delivery, DeliveryStatus, NewRun, Run, RunStatus, VehicleType, WlDelivery};
use wl_kernel::documents::today;

use support::documents;
use support::roster::employee_of;
use support::{VAN, World, boot_custody_with, boot_with_custody, stop_fields};

/// A run of `driver` with `deliveries` on it, dispatched today.
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

/// A planned run of `driver` for today, no stops.
async fn planned_run(w: &World, driver: &User) -> Run {
    w.valid_documents_for(driver.id, "2099-12-31");
    wl_delivery::plan::create_run(
        w.state(),
        NewRun::new(today(), driver.id).vehicle(VehicleType::Van, Some(VAN)),
    )
    .await
    .unwrap()
}

/// The giving run R1 of the driver: a delivery of AED 400 cash on delivery
/// (delivered by the driver, so AED 400 is in the driver's custody) and
/// `open` further stops, all on the run, dispatched. Returns the run and the
/// open stops.
async fn giving_run(w: &World, open: usize) -> (Run, Vec<Delivery>) {
    let cash = w.request(&[("Tiles", 10_000, None)], 40_000).await;
    let mut stops = Vec::new();
    for _ in 0..open {
        stops.push(w.simple_delivery().await);
    }
    let mut all: Vec<&Delivery> = vec![&cash];
    all.extend(stops.iter());
    let run = dispatched_run(w, &w.driver, &all).await;
    let line = w.lines(&cash).await.remove(0);
    let fields = stop_fields(&[(line.id, "10", "0", "", "")], "400.00");
    w.deliver(&w.driver, &cash, &fields)
        .await
        .assert_status(303);
    let mut reloaded = Vec::new();
    for stop in &stops {
        reloaded.push(w.reload(stop).await);
    }
    (run, reloaded)
}

fn hand_over_fields(to_run: &Run, reason: &str, stops: &[&Delivery]) -> Vec<(String, String)> {
    let mut fields = vec![
        ("to_run_id".to_owned(), to_run.id.to_string()),
        ("reason".to_owned(), reason.to_owned()),
    ];
    for stop in stops {
        fields.push((format!("delivery_{}", stop.id), "on".to_owned()));
    }
    fields
}

/// The dispatcher proposes the hand-over of `stops` of `run` to `to_run`.
async fn propose(
    w: &World,
    run: &Run,
    to_run: &Run,
    reason: &str,
    stops: &[&Delivery],
) -> renox::testing::TestResponse {
    let owned = hand_over_fields(to_run, reason, stops);
    let fields: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/hand-over", run.id),
        &fields,
    )
    .await
}

/// The receiving driver confirms `parcels` and counts `cash` (AED text, or
/// none).
async fn confirm(
    w: &World,
    by: &User,
    hand_over: &HandOver,
    parcels: &[&Delivery],
    cash: Option<&str>,
) -> renox::testing::TestResponse {
    let mut owned: Vec<(String, String)> = parcels
        .iter()
        .map(|d| (format!("parcel_{}", d.id), "on".to_owned()))
        .collect();
    if let Some(cash) = cash {
        owned.push(("cash".to_owned(), cash.to_owned()));
    }
    let fields: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(
        by,
        &format!("/driver/hand-overs/{}/confirm", hand_over.id),
        &fields,
    )
    .await
}

async fn only_hand_over(w: &World, run: &Run) -> HandOver {
    let all = HandOver::of_run(w.db(), run.id).await.unwrap();
    assert_eq!(all.len(), 1, "one hand-over");
    all.into_iter().next().unwrap()
}

/// AC1: R1 (driver A, AED 400 collected, 5 open stops) and R2 (driver B,
/// dispatched, documents valid): the dispatcher hands over 3 stops with
/// reason *breakdown* and B scans 3 parcels and counts AED 400: the 3
/// deliveries are on R2 and `out_for_delivery`, the 2 others stay, the
/// hand-over record is confirmed and `RunHandedOver` is emitted with the
/// counted cash and the stops.
#[renox::test]
async fn three_stops_move_to_the_other_run_when_the_receiver_confirms() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 5).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let chosen = [&stops[0], &stops[1], &stops[2]];

    propose(&w, &r1, &r2, "breakdown", &chosen)
        .await
        .assert_ok();
    // Nothing moves until the receiving driver confirms.
    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));
    assert!(w.handed_over.all().is_empty());
    let waiting = only_hand_over(&w, &r1).await;
    assert!(!waiting.is_confirmed());

    confirm(&w, &w.other_driver, &waiting, &chosen, Some("400.00"))
        .await
        .assert_status(303);

    for stop in chosen {
        let moved = w.reload(stop).await;
        assert_eq!(moved.run_id, Some(r2.id));
        assert_eq!(moved.status, DeliveryStatus::OutForDelivery);
    }
    for stop in [&stops[3], &stops[4]] {
        let kept = w.reload(stop).await;
        assert_eq!(kept.run_id, Some(r1.id));
        assert_eq!(kept.status, DeliveryStatus::OutForDelivery);
    }
    assert_eq!(w.reload_run(&r1).await.status, RunStatus::Dispatched);
    let record = only_hand_over(&w, &r1).await;
    assert!(record.is_confirmed());
    assert_eq!(record.confirmed_by, Some(w.other_driver.id));
    assert_eq!(record.cash_counted, Some(40_000));
    let events = w.handed_over.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].run, r1.run_id());
    assert_eq!(events[0].to_run, r2.run_id());
    assert_eq!(events[0].from_driver, w.driver.id);
    assert_eq!(events[0].to_driver, w.other_driver.id);
    assert_eq!(events[0].requested_by, w.dispatcher.id);
    assert_eq!(events[0].reason, "breakdown");
    assert_eq!(events[0].cash_counted, 40_000);
    assert_eq!(events[0].deliveries.len(), 3);
    // `DeliveryAssigned` for each stop that moved, on the receiving run.
    let assigned = w.assigned.all();
    let on_r2: Vec<_> = assigned.iter().filter(|e| e.run == r2.run_id()).collect();
    assert_eq!(on_r2.len(), 3);
    assert!(on_r2.iter().all(|e| e.driver == w.other_driver.id));
    // The moved stops are B's now: B can deliver one.
    let line = w.lines(&stops[0]).await.remove(0);
    w.deliver(
        &w.other_driver,
        &stops[0],
        &stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&stops[0]).await.status, DeliveryStatus::Delivered);
}

/// AC2: B's rider permit expired yesterday: the hand-over is refused with the
/// document named, nothing is created and no stop moves.
#[renox::test]
async fn a_receiving_driver_with_an_expired_permit_is_refused() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let yesterday = today() - Days::new(1);
    w.docs.set(
        &employee_of(w.other_driver.id).subject(),
        documents::RIDER_PERMIT,
        Some(yesterday),
    );

    propose(&w, &r1, &r2, "breakdown", &[&stops[0]])
        .await
        .assert_status(422)
        .assert_see("Rider permit expired on");

    assert!(HandOver::of_run(w.db(), r1.id).await.unwrap().is_empty());
    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));
    assert!(w.handed_over.all().is_empty());
}

/// The document check runs again at the confirmation: a permit that expired
/// between the proposal and the confirmation stops the hand-over.
#[renox::test]
async fn a_permit_that_expires_before_the_confirmation_stops_the_hand_over() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    propose(&w, &r1, &r2, "accident", &[&stops[0]])
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;
    w.docs.set(
        &employee_of(w.other_driver.id).subject(),
        documents::RIDER_PERMIT,
        Some(today() - Days::new(1)),
    );

    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], Some("400.00"))
        .await
        .assert_status(422)
        .assert_see("Rider permit expired on");

    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));
    assert!(!only_hand_over(&w, &r1).await.is_confirmed());
    assert!(w.handed_over.all().is_empty());
}

/// AC2 (the vehicle half of the guard): the receiving run's vehicle has an
/// expired insurance: the hand-over is refused at the proposal with the
/// document named, nothing is created and no stop moves; and an insurance that
/// expires before the confirmation stops the hand-over there too.
#[renox::test]
async fn a_receiving_vehicle_with_an_expired_insurance_stops_the_hand_over() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let yesterday = today() - Days::new(1);
    w.docs.set(
        &w.vehicle_subject(),
        documents::INSURANCE,
        Some(yesterday),
    );

    propose(&w, &r1, &r2, "breakdown", &[&stops[0]])
        .await
        .assert_status(422)
        .assert_see("expired on");
    assert!(HandOver::of_run(w.db(), r1.id).await.unwrap().is_empty());
    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));

    // Valid at the proposal, expired at the confirmation.
    w.docs.set(
        &w.vehicle_subject(),
        documents::INSURANCE,
        Some(today() + Days::new(30)),
    );
    propose(&w, &r1, &r2, "breakdown", &[&stops[0]])
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;
    w.docs.set(
        &w.vehicle_subject(),
        documents::INSURANCE,
        Some(yesterday),
    );
    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], Some("400.00"))
        .await
        .assert_status(422)
        .assert_see("expired on");
    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));
    assert!(!only_hand_over(&w, &r1).await.is_confirmed());
    assert!(w.handed_over.all().is_empty());
}

/// AC3: B counts AED 380 while A's custody holds AED 400: `RunHandedOver`
/// carries the AED 380 counted, and the giving driver's custody is not
/// touched here, so the AED 20 stays in A's handover as the shortage the
/// count will find (the move itself is `wl-payments`').
#[renox::test]
async fn the_counted_amount_travels_and_the_difference_stays_with_the_giver() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 3).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let chosen = [&stops[0], &stops[1], &stops[2]];
    propose(&w, &r1, &r2, "breakdown", &chosen)
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;

    confirm(&w, &w.other_driver, &waiting, &chosen, Some("380.00"))
        .await
        .assert_status(303);

    let events = w.handed_over.all();
    assert_eq!(events[0].cash_counted, 38_000);
    assert_eq!(w.custody.expected(w.driver.id, &r1.subject()), 40_000);
}

/// AC4: R1 with every stop delivered or handed over: A can `return_to_depot`;
/// with a stop still open it is refused.
#[renox::test]
async fn the_giving_driver_returns_once_every_stop_is_done_or_handed_over() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    // Both open: refused.
    w.post(&w.driver, &format!("/driver/runs/{}/return", r1.id), &[])
        .await
        .assert_status(422);

    propose(&w, &r1, &r2, "driver_unwell", &[&stops[0]])
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;
    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], Some("0"))
        .await
        .assert_status(303);
    // One stop is still open on R1.
    w.post(&w.driver, &format!("/driver/runs/{}/return", r1.id), &[])
        .await
        .assert_status(422);
    let line = w.lines(&stops[1]).await.remove(0);
    w.deliver(
        &w.driver,
        &stops[1],
        &stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await
    .assert_status(303);

    w.post(&w.driver, &format!("/driver/runs/{}/return", r1.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        w.reload_run(&r1).await.status,
        RunStatus::ReturnedToDepot,
        "the AED 400 handover is still to be counted"
    );
}

/// Must not: cash and stops do not move without the receiving driver's
/// confirmation: the giving driver, the dispatcher and a driver who is not on
/// the receiving run cannot confirm, a parcel left unconfirmed or cash left
/// uncounted refuses it, and a hand-over confirms only once.
#[renox::test]
async fn only_the_receiving_driver_confirms_and_only_with_every_parcel_and_the_cash() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let chosen = [&stops[0], &stops[1]];
    propose(&w, &r1, &r2, "breakdown", &chosen)
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;

    // The giving driver cannot take the stops over.
    confirm(&w, &w.driver, &waiting, &chosen, Some("400.00"))
        .await
        .assert_status(422)
        .assert_see("receiving run");
    // The dispatcher holds no driving permission.
    confirm(&w, &w.dispatcher, &waiting, &chosen, Some("400.00"))
        .await
        .assert_status(403);
    // A parcel is missing.
    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], Some("400.00"))
        .await
        .assert_status(422)
        .assert_see("1 left");
    // The cash is not counted.
    confirm(&w, &w.other_driver, &waiting, &chosen, None)
        .await
        .assert_status(422)
        .assert_see("Count the cash");
    assert_eq!(w.reload(&stops[0]).await.run_id, Some(r1.id));
    assert!(w.handed_over.all().is_empty());

    confirm(&w, &w.other_driver, &waiting, &chosen, Some("400.00"))
        .await
        .assert_status(303);
    confirm(&w, &w.other_driver, &waiting, &chosen, Some("400.00"))
        .await
        .assert_status(422)
        .assert_see("already confirmed");
    assert_eq!(w.handed_over.all().len(), 1);
}

/// Must not: a hand-over to the same driver, to the same run, with an unknown
/// reason, with no stop or with a stop that is not an open stop of the run is
/// refused; a driver cannot propose one.
#[renox::test]
async fn a_hand_over_to_the_same_driver_or_with_a_bad_choice_is_refused() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 2).await;
    let same_driver = dispatched_run(&w, &w.driver, &[]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let other = w.simple_delivery().await;

    propose(&w, &r1, &same_driver, "breakdown", &[&stops[0]])
        .await
        .assert_status(422)
        .assert_see("another driver");
    propose(&w, &r1, &r1, "breakdown", &[&stops[0]])
        .await
        .assert_status(422)
        .assert_see("another run");
    propose(&w, &r1, &r2, "boredom", &[&stops[0]])
        .await
        .assert_status(422)
        .assert_see("Choose why");
    propose(&w, &r1, &r2, "breakdown", &[])
        .await
        .assert_status(422)
        .assert_see("at least one");
    propose(&w, &r1, &r2, "breakdown", &[&other])
        .await
        .assert_status(422)
        .assert_see("not an open stop");
    let owned = hand_over_fields(&r2, "breakdown", &[&stops[0]]);
    let fields: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(&w.driver, &format!("/runs/{}/hand-over", r1.id), &fields)
        .await
        .assert_status(403);
    assert!(HandOver::of_run(w.db(), r1.id).await.unwrap().is_empty());
}

/// Edge: a stop delivered on the giving driver's phone while the hand-over
/// waits is skipped; only the stops still open at the confirmation move.
#[renox::test]
async fn a_stop_delivered_meanwhile_is_left_out_of_the_hand_over() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 3).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    let chosen = [&stops[0], &stops[1], &stops[2]];
    propose(&w, &r1, &r2, "load_balancing", &chosen)
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;
    let line = w.lines(&stops[0]).await.remove(0);
    w.deliver(
        &w.driver,
        &stops[0],
        &stop_fields(&[(line.id, "10", "0", "", "")], "0"),
    )
    .await
    .assert_status(303);

    confirm(
        &w,
        &w.other_driver,
        &waiting,
        &[&stops[1], &stops[2]],
        Some("0"),
    )
    .await
    .assert_status(303);

    let delivered = w.reload(&stops[0]).await;
    assert_eq!(delivered.status, DeliveryStatus::Delivered);
    assert_eq!(delivered.run_id, Some(r1.id));
    assert_eq!(w.reload(&stops[1]).await.run_id, Some(r2.id));
    assert_eq!(w.reload(&stops[2]).await.run_id, Some(r2.id));
    assert_eq!(w.handed_over.all()[0].deliveries.len(), 2);
}

/// Edge: a receiving run that is still `planned` takes the stops: they stay
/// `out_for_delivery`, and its dispatch leaves them as they are.
#[renox::test]
async fn a_planned_receiving_run_keeps_the_handed_over_stops_out_when_it_is_dispatched() {
    let w = boot_with_custody().await;
    let (r1, stops) = giving_run(&w, 1).await;
    let r2 = planned_run(&w, &w.other_driver).await;
    propose(&w, &r1, &r2, "accident", &[&stops[0]])
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;
    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], Some("400.00"))
        .await
        .assert_status(303);
    let moved = w.reload(&stops[0]).await;
    assert_eq!(moved.run_id, Some(r2.id));
    assert_eq!(moved.status, DeliveryStatus::OutForDelivery);

    w.dispatch(&w.dispatcher, &r2).await.assert_ok();

    assert_eq!(
        w.reload(&stops[0]).await.status,
        DeliveryStatus::OutForDelivery
    );
    assert_eq!(w.reload_run(&r2).await.status, RunStatus::Dispatched);
}

/// Settings off: with the count off the amount recorded in the giving
/// driver's custody moves as it is.
#[renox::test]
async fn with_the_count_off_the_recorded_custody_amount_moves() {
    let w = boot_custody_with(WlDelivery::new().hand_over_counts_cash(false)).await;
    let (r1, stops) = giving_run(&w, 1).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;
    propose(&w, &r1, &r2, "breakdown", &[&stops[0]])
        .await
        .assert_ok();
    let waiting = only_hand_over(&w, &r1).await;

    confirm(&w, &w.other_driver, &waiting, &[&stops[0]], None)
        .await
        .assert_status(303);

    assert_eq!(w.handed_over.all()[0].cash_counted, 40_000);
}
