//! BL-DLV-01 (#157), `WRK.DSP.01`, `X-13`: `assign` and run `dispatch` are
//! refused on an invalid driver or vehicle document. Which kinds block comes
//! from the document kinds' `blocks` (`delivery.assign` on the run's date,
//! `run.dispatch` today); for 03 they are legal blocks with no override.

mod support;

use renox::chrono::Days;
use wl_delivery::{DeliveryStatus, RunStatus, WlDelivery};
use wl_kernel::documents::{OverrideMode, today};

use support::documents::{self, Documents};
use support::{boot, boot_with, date};

fn day_text(day: renox::chrono::NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

/// AC1: driver A's licence expires 2026-11-10 and the run is planned for
/// 2026-11-12: `assign` is refused with the licence named and its expiry
/// date, and the delivery stays pending.
#[renox::test]
async fn assign_is_refused_when_the_licence_expires_before_the_run_date() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    w.docs.set(
        &w.driver_subject(),
        documents::LICENCE,
        Some(date("2026-11-10")),
    );
    let run = w.run_on("2026-11-12").await;
    let delivery = w.simple_delivery().await;

    w.assign(&w.dispatcher, &delivery, &run)
        .await
        .assert_status(422)
        .assert_see("Driving licence expired on 2026-11-10")
        .assert_see("renew: HR");

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Pending);
    assert_eq!(after.run_id, None);
    assert!(w.assigned.all().is_empty());
}

/// AC2: the same driver and a run planned for 2026-11-09: `assign` succeeds,
/// the licence being valid through 2026-11-10.
#[renox::test]
async fn assign_succeeds_when_the_licence_is_valid_on_the_run_date() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    w.docs.set(
        &w.driver_subject(),
        documents::LICENCE,
        Some(date("2026-11-10")),
    );
    let run = w.run_on("2026-11-09").await;
    let delivery = w.simple_delivery().await;

    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Assigned);
    assert_eq!(after.run_id, Some(run.id));
    assert_eq!(w.assigned.all().len(), 1);
}

/// AC3: a planned run whose vehicle insurance expired yesterday: `dispatch`
/// is refused, the run stays planned, and no delivery moves to
/// `out_for_delivery`.
#[renox::test]
async fn dispatch_is_refused_when_the_insurance_expired_yesterday() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let day = today();
    let run = w.run_on(&day_text(day)).await;
    let delivery = w.simple_delivery().await;
    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    // The insurance expires between `assign` and `dispatch`: the deliveries
    // are not unassigned, the dispatch is refused.
    let yesterday = day - Days::new(1);
    w.docs
        .set(&w.vehicle_subject(), documents::INSURANCE, Some(yesterday));

    w.dispatch(&w.dispatcher, &run)
        .await
        .assert_status(422)
        .assert_see(&format!(
            "Vehicle insurance expired on {}",
            day_text(yesterday)
        ));

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Planned);
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Assigned);
    assert!(w.out.all().is_empty());
    assert!(w.dispatched.all().is_empty());
}

/// AC4: an on-foot rider with a valid rider permit and no vehicle: `dispatch`
/// succeeds, only the driver's kinds being checked.
#[renox::test]
async fn an_on_foot_rider_with_a_valid_permit_is_dispatched_without_a_vehicle() {
    let w = boot().await;
    let far = Some(date("2099-12-31"));
    w.docs.set(&w.driver_subject(), documents::LICENCE, far);
    w.docs
        .set(&w.driver_subject(), documents::RIDER_PERMIT, far);
    // No Mulkiya and no insurance exist anywhere: a vehicle check would fail.
    let run = w.on_foot_run_on(&day_text(today())).await;
    let delivery = w.simple_delivery().await;
    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    w.dispatch(&w.dispatcher, &run).await.assert_ok();

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Dispatched);
    assert_eq!(
        w.reload(&delivery).await.status,
        DeliveryStatus::OutForDelivery
    );
    let events = w.dispatched.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].vehicle, None);
}

/// AC5: a 03 client (override mode `never`): the refusal offers no override
/// request.
#[renox::test]
async fn a_legal_block_offers_no_override_request() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    w.docs.set(
        &w.vehicle_subject(),
        documents::MULKIYA,
        Some(date("2026-01-01")),
    );
    let run = w.run_on("2026-11-12").await;
    let delivery = w.simple_delivery().await;

    let refused = w.assign(&w.dispatcher, &delivery, &run).await;
    refused
        .assert_status(422)
        .assert_see("Mulkiya expired on 2026-01-01")
        .assert_see("override: not allowed")
        .assert_dont_see("overrides/new");
}

/// AC5, the other side: a kind with the override mode `with_approval` offers
/// the request, so the absence above is the `never` mode and not a missing
/// feature.
#[renox::test]
async fn a_kind_that_allows_an_override_offers_the_request() {
    let docs = Documents::none();
    docs.block(
        documents::INSURANCE,
        "run.dispatch",
        OverrideMode::WithApproval,
        false,
    );
    let w = boot_with(WlDelivery::new(), docs).await;
    w.docs.set(
        &w.vehicle_subject(),
        documents::INSURANCE,
        Some(date("2026-01-01")),
    );
    let run = w.run_on(&day_text(today())).await;

    w.dispatch(&w.dispatcher, &run)
        .await
        .assert_status(422)
        .assert_see("Vehicle insurance expired on 2026-01-01")
        .assert_see("overrides/new?kind=vehicle_insurance");
    assert_eq!(w.reload_run(&run).await.status, RunStatus::Planned);
}

/// AC6: no document kind lists `delivery.assign`: `assign` succeeds without
/// asking `document_valid` (the licence does not even exist).
#[renox::test]
async fn assign_succeeds_without_asking_when_no_kind_lists_the_transition() {
    let w = boot_with(WlDelivery::new(), Documents::none()).await;
    let run = w.run_on("2026-11-12").await;
    let delivery = w.simple_delivery().await;

    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Assigned);
    assert_eq!(w.docs.asked(), 0);
}

/// Edge case: `dispatch` is checked today, not on the run's date. The run was
/// planned for yesterday, the licence expired yesterday: `assign` (checked on
/// the run's date) passed, `dispatch` is refused.
#[renox::test]
async fn dispatch_is_checked_today_and_not_on_the_run_date() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let yesterday = today() - Days::new(1);
    w.docs
        .set(&w.driver_subject(), documents::LICENCE, Some(yesterday));
    let run = w.run_on(&day_text(yesterday)).await;
    let delivery = w.simple_delivery().await;

    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();
    w.dispatch(&w.dispatcher, &run)
        .await
        .assert_status(422)
        .assert_see("Driving licence expired on");

    assert_eq!(w.reload_run(&run).await.status, RunStatus::Planned);
}

/// Edge case: a rider permit of the driver and an insurance of the vehicle
/// are each checked on their own subject: the vehicle's insurance is not
/// asked of the driver, and the licence is not asked of the vehicle.
#[renox::test]
async fn each_kind_is_checked_on_the_subject_it_attaches_to() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    // A licence on the vehicle subject is irrelevant, as is an insurance on
    // the driver: only the right subjects count.
    w.docs.remove(&w.driver_subject(), documents::INSURANCE);
    w.docs.remove(&w.vehicle_subject(), documents::LICENCE);
    let run = w.run_on("2026-11-12").await;
    let delivery = w.simple_delivery().await;
    w.assign(&w.dispatcher, &delivery, &run).await.assert_ok();

    // A missing licence of the driver refuses, and names the kind.
    w.docs.remove(&w.driver_subject(), documents::LICENCE);
    let second = w.simple_delivery().await;
    w.assign(&w.dispatcher, &second, &run)
        .await
        .assert_status(422)
        .assert_see("Driving licence missing");
}

/// Edge case: the driver is on leave on the run's date: `assign` is refused
/// ("the driver is available"), and not when the leave ends before.
#[renox::test]
async fn assign_is_refused_while_the_driver_is_on_leave() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    w.roster.on_leave(w.driver.id, date("2026-12-01"));
    let delivery = w.simple_delivery().await;

    let during = w.run_on("2026-11-20").await;
    w.assign(&w.dispatcher, &delivery, &during)
        .await
        .assert_status(422)
        .assert_see("on leave");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Pending);

    let after = w.run_on("2026-12-02").await;
    w.assign(&w.dispatcher, &delivery, &after).await.assert_ok();
}

/// Edge case: only a dispatcher assigns; a driver cannot.
#[renox::test]
async fn only_a_holder_of_deliveries_dispatch_assigns() {
    let w = boot().await;
    w.valid_documents("2099-12-31");
    let run = w.run_on("2026-11-12").await;
    let delivery = w.simple_delivery().await;
    w.assign(&w.driver, &delivery, &run)
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Pending);
}
