//! BL-DLV-09 (#165) and BL-PAY-20 (#113), `FIN.CSH.01`, X-12: a run
//! hand-over with the real `wl-payments` as the cash custody. The receiving
//! driver's count moves the cash from the giving driver's handover of the run
//! to the receiving driver's handover of the receiving run, and a difference
//! stays on the giver and shows as a shortage at its count.

mod support;

use renox::prelude::*;
use wl_delivery::hand_over::HandOver;
use wl_delivery::{Delivery, NewRun, Run, RunStatus, VehicleType, close};
use wl_kernel::custody::{HandoverStatus, HandoverSummary};
use wl_kernel::documents::today;
use wl_payments::cash_hand_over::OVERAGE_SOURCE_TYPE;
use wl_payments::cash_handover::CashEntry;
use wl_payments::cash_notices::CASH_HANDED_OVER;
use wl_payments::permissions as pay;

use support::{VAN, World, boot_with_real_payments, stop_fields, user_with};

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

/// R1 of driver A with a delivery of AED 400 cash on delivery already
/// delivered (AED 400 in A's handover of the run) and `open` open stops, and
/// R2 of driver B, dispatched.
async fn two_runs(w: &World, open: usize) -> (Run, Vec<Delivery>, Run) {
    let cash = w.request(&[("Tiles", 10_000, None)], 40_000).await;
    let mut stops = Vec::new();
    for _ in 0..open {
        stops.push(w.simple_delivery().await);
    }
    let mut all: Vec<&Delivery> = vec![&cash];
    all.extend(stops.iter());
    let r1 = dispatched_run(w, &w.driver, &all).await;
    let line = w.lines(&cash).await.remove(0);
    w.deliver(
        &w.driver,
        &cash,
        &stop_fields(&[(line.id, "10", "0", "", "")], "400.00"),
    )
    .await
    .assert_status(303);
    let r2 = dispatched_run(w, &w.other_driver, &[]).await;
    (r1, stops, r2)
}

/// The dispatcher proposes `stops`, B confirms every parcel and counts
/// `cash` AED.
async fn hand_over(w: &World, r1: &Run, r2: &Run, stops: &[&Delivery], cash: &str) {
    let mut owned = vec![
        ("to_run_id".to_owned(), r2.id.to_string()),
        ("reason".to_owned(), "breakdown".to_owned()),
    ];
    for stop in stops {
        owned.push((format!("delivery_{}", stop.id), "on".to_owned()));
    }
    let fields: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.post(
        &w.dispatcher,
        &format!("/runs/{}/hand-over", r1.id),
        &fields,
    )
    .await
    .assert_ok();
    let record = HandOver::of_run(w.db(), r1.id)
        .await
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    let mut parcels: Vec<(String, String)> = stops
        .iter()
        .map(|d| (format!("parcel_{}", d.id), "on".to_owned()))
        .collect();
    parcels.push(("cash".to_owned(), cash.to_owned()));
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

async fn custody_of(w: &World, run: &Run) -> Option<HandoverSummary> {
    close::handover_of(w.state(), run).await.unwrap()
}

/// AC1 of #165: R1 (A, AED 400 collected, 5 open stops) and R2 (B): the
/// dispatcher hands over 3 stops with *breakdown* and B scans 3 parcels and
/// counts AED 400: the 3 deliveries are on R2 and out, B's custody is +400
/// for R2, A's handover of R1 has nothing left to hand over, and
/// `RunHandedOver` is emitted; both drivers and the dispatcher are told.
#[renox::test]
async fn the_cash_a_receiving_driver_counts_moves_into_his_custody() {
    let w = boot_with_real_payments().await;
    let (r1, stops, r2) = two_runs(&w, 5).await;
    assert_eq!(custody_of(&w, &r1).await.unwrap().expected, 40_000);
    assert!(custody_of(&w, &r2).await.is_none());

    hand_over(&w, &r1, &r2, &[&stops[0], &stops[1], &stops[2]], "400.00").await;

    for stop in &stops[..3] {
        let moved = w.reload(stop).await;
        assert_eq!(moved.run_id, Some(r2.id));
        assert_eq!(moved.status.as_str(), "out_for_delivery");
    }
    let theirs = custody_of(&w, &r2).await.expect("opened for B");
    assert_eq!(theirs.expected, 40_000);
    assert_eq!(theirs.status, HandoverStatus::Open);
    assert_eq!(theirs.holder_user, w.other_driver.id);
    let ours = custody_of(&w, &r1).await.unwrap();
    assert_eq!(ours.expected, 0);
    assert_eq!(ours.holder_user, w.driver.id);
    // Nothing is left to hand over; the handover is not counted by itself,
    // it stays open at 0 until the depot counts it (DEFERRED-dlv, #165).
    assert_eq!(ours.status, HandoverStatus::Open);
    assert_eq!(w.handed_over.all().len(), 1);
    let told = w.notices.users_of_type(CASH_HANDED_OVER);
    assert!(told.contains(&w.driver.id));
    assert!(told.contains(&w.other_driver.id));
    assert!(told.contains(&w.dispatcher.id));
}

/// AC3 of #165 and AC1-AC2 of #113: B counts AED 380: B's custody is +380 and
/// A keeps AED 20 expected; at A's count with nothing counted, A is short
/// AED 20.00.
#[renox::test]
async fn a_difference_stays_with_the_giver_and_is_a_shortage_at_its_count() {
    let w = boot_with_real_payments().await;
    let (r1, stops, r2) = two_runs(&w, 4).await;
    hand_over(&w, &r1, &r2, &[&stops[0], &stops[1], &stops[2]], "380.00").await;

    assert_eq!(custody_of(&w, &r2).await.unwrap().expected, 38_000);
    assert_eq!(custody_of(&w, &r1).await.unwrap().expected, 2_000);

    // A finishes the last stop, comes back and the depot counts nothing.
    w.fail(&w.driver, &stops[3], "not_home")
        .await
        .assert_status(303);
    w.post(&w.driver, &format!("/driver/runs/{}/return", r1.id), &[])
        .await
        .assert_ok();
    let counter = user_with(&w, "Chandra", &[pay::CASH_VIEW, pay::CASH_COUNT]).await;
    let ours = custody_of(&w, &r1).await.unwrap();
    w.post(
        &counter,
        &format!("/cash/{}/count", ours.id),
        &[("counted", "0.00")],
    )
    .await
    .assert_ok();

    let after = custody_of(&w, &r1).await.unwrap();
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!(after.difference, 2_000);
    // The run still closes on the count, once its stop is received back.
    assert_eq!(w.reload_run(&r1).await.status, RunStatus::ReturnedToDepot);
}

/// AC3 of #113 with the real delivery: B counts AED 430 against AED 400: B's
/// handover expects AED 430 with an overage line of AED 30, A's expects
/// nothing.
#[renox::test]
async fn an_excess_counted_by_the_receiver_is_an_overage_line() {
    let w = boot_with_real_payments().await;
    let (r1, stops, r2) = two_runs(&w, 1).await;

    hand_over(&w, &r1, &r2, &[&stops[0]], "430.00").await;

    let theirs = custody_of(&w, &r2).await.unwrap();
    assert_eq!(theirs.expected, 43_000);
    let ours = custody_of(&w, &r1).await.unwrap();
    assert_eq!(ours.expected, 0);
    assert_eq!(ours.status, HandoverStatus::Open);
    let entries = CashEntry::of_handover(w.db(), theirs.id.0).await.unwrap();
    assert_eq!(entries.len(), 2);
    let line = entries
        .iter()
        .find(|e| e.source_type == OVERAGE_SOURCE_TYPE)
        .expect("the overage line");
    assert_eq!(line.amount, 3_000);
}
