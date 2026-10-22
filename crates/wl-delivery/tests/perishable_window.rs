//! BL-DLV-10 (#166), X-12, 06-N9-b: a perishable stop is handed over to
//! another run only when the receiving run can still deliver it within the
//! hand-over window (the source's placed time plus its delivery promise); the
//! dispatcher enters the expected arrival of each perishable stop and a late
//! one is refused and stays on the giving run. Non-perishable stops need no
//! arrival.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use renox::prelude::*;
use wl_delivery::hand_over::{HandOver, HandOverStop};
use wl_delivery::{Deliveries, Delivery, DeliveryStatus, NewRun, Run, VehicleType};
use wl_kernel::documents::today;
use wl_kernel::{DeliveryItem, DeliveryRequest, Subject};

use support::{VAN, World, boot};

static SOURCE: AtomicI64 = AtomicI64::new(8_000);

/// An F&B delivery placed at 19:00 Dubai time today with a 45-minute promise.
async fn food(w: &World) -> Delivery {
    let source = SOURCE.fetch_add(1, Ordering::SeqCst);
    let placed = today().and_hms_opt(15, 0, 0).unwrap().and_utc();
    let request = DeliveryRequest::new(Subject::new("fnb.order", source), "Mariam Al Hosani")
        .address("Villa 12, Al Barsha 2", "Al Barsha")
        .phone("+971500000000")
        .item(DeliveryItem::new("Chicken meal", 2_000))
        .promise(placed, 45)
        .perishable();
    Deliveries::request(w.state(), request).await.unwrap()
}

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

/// The dispatcher proposes to hand `stops` over, with an expected arrival
/// per stop that has one.
async fn propose(
    w: &World,
    run: &Run,
    to_run: &Run,
    stops: &[(&Delivery, Option<&str>)],
) -> renox::testing::TestResponse {
    let mut owned = vec![
        ("to_run_id".to_owned(), to_run.id.to_string()),
        ("reason".to_owned(), "breakdown".to_owned()),
    ];
    for (stop, arrival) in stops {
        owned.push((format!("delivery_{}", stop.id), "on".to_owned()));
        if let Some(arrival) = arrival {
            owned.push((format!("arrival_{}", stop.id), (*arrival).to_owned()));
        }
    }
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

async fn proposals(w: &World, run: &Run) -> Vec<HandOver> {
    HandOver::of_run(w.db(), run.id).await.unwrap()
}

/// AC1: an F&B delivery placed 19:00 with a 45-minute promise, handed over
/// with an expected arrival of 19:40, is accepted.
#[renox::test]
async fn a_perishable_stop_that_can_arrive_within_the_window_is_accepted() {
    let w = boot().await;
    let meal = food(&w).await;
    let r1 = dispatched_run(&w, &w.driver, &[&meal]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&meal, Some("19:40"))])
        .await
        .assert_ok();

    let made = proposals(&w, &r1).await;
    assert_eq!(made.len(), 1);
    let stops = HandOverStop::query()
        .where_(HandOverStop::HAND_OVER_ID.eq(made[0].id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(stops.len(), 1);
    let expected = today().and_hms_opt(15, 40, 0).unwrap().and_utc();
    assert_eq!(stops[0].expected_arrival, Some(expected));
}

/// An arrival exactly at the window end is accepted.
#[renox::test]
async fn an_arrival_exactly_at_the_window_end_is_accepted() {
    let w = boot().await;
    let meal = food(&w).await;
    let r1 = dispatched_run(&w, &w.driver, &[&meal]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&meal, Some("19:45"))])
        .await
        .assert_ok();

    assert_eq!(proposals(&w, &r1).await.len(), 1);
}

/// AC2: with an expected arrival of 19:50 the stop is refused and stays on the
/// giving run.
#[renox::test]
async fn a_perishable_stop_that_would_arrive_after_the_window_is_refused_and_stays() {
    let w = boot().await;
    let meal = food(&w).await;
    let r1 = dispatched_run(&w, &w.driver, &[&meal]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&meal, Some("19:50"))])
        .await
        .assert_status(422)
        .assert_see("cannot arrive by");

    assert!(proposals(&w, &r1).await.is_empty());
    let stays = w.reload(&meal).await;
    assert_eq!(stays.status, DeliveryStatus::OutForDelivery);
    assert_eq!(stays.run_id, Some(r1.id));
}

/// A perishable stop is never handed over without an expected arrival.
#[renox::test]
async fn a_perishable_stop_needs_an_expected_arrival() {
    let w = boot().await;
    let meal = food(&w).await;
    let r1 = dispatched_run(&w, &w.driver, &[&meal]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&meal, None)])
        .await
        .assert_status(422)
        .assert_see("expected arrival");
    propose(&w, &r1, &r2, &[(&meal, Some("soon"))])
        .await
        .assert_status(422);

    assert!(proposals(&w, &r1).await.is_empty());
}

/// AC3: non-perishable stops are handed over without an expected arrival.
#[renox::test]
async fn a_stop_that_is_not_perishable_needs_no_arrival() {
    let w = boot().await;
    let tiles = w.simple_delivery().await;
    let r1 = dispatched_run(&w, &w.driver, &[&tiles]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&tiles, None)]).await.assert_ok();

    assert_eq!(proposals(&w, &r1).await.len(), 1);
}

/// One late perishable stop refuses the whole proposal, so nothing moves;
/// the stop stays on the giving run, where an abort fails it with *run
/// aborted* and `WRK.RUL.16` applies.
#[renox::test]
async fn a_refused_stop_stays_on_the_giving_run_where_an_abort_applies_the_perishable_policy() {
    let w = boot().await;
    let meal = food(&w).await;
    let tiles = w.simple_delivery().await;
    let r1 = dispatched_run(&w, &w.driver, &[&meal, &tiles]).await;
    let r2 = dispatched_run(&w, &w.other_driver, &[]).await;

    propose(&w, &r1, &r2, &[(&meal, Some("20:30")), (&tiles, None)])
        .await
        .assert_status(422);
    assert!(proposals(&w, &r1).await.is_empty());

    w.post(
        &w.dispatcher,
        &format!("/runs/{}/abort", r1.id),
        &[("reason", "breakdown")],
    )
    .await
    .assert_ok();

    let failed = w.reload(&meal).await;
    assert_eq!(failed.status, DeliveryStatus::Attempted);
    assert_eq!(failed.resolution.as_deref(), Some("discard"));
}
