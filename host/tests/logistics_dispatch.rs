//! BL-LOG-25 (#323) *Documents* and BL-LOG-26 (#324) *Delivery*, 03 §3.3,
//! §9, `WRK.ACL.09`, `FIN.CSH.01`: the 03 values run through the real
//! routes of a fresh 03 deployment (the host builders, the roles of the host,
//! the seeded document kinds), not through a copy of the configuration: a
//! dispatch with an expired Mulkiya is refused by the dispatch route with no
//! override, and a cash on delivery collected AED 0.01 short asks the
//! operations manager, who may not choose the customer; unanswered for ten
//! minutes it lapses and leaves only the failed attempt.

mod support;

use host::roles::{DISPATCHER, DRIVER, OPERATIONS_MANAGER};
use logistics::defaults;
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_delivery::{Deliveries, Delivery, DeliveryStatus, Run, RunStatus, VehicleType, cod};
use wl_documents::{Document, DocumentKind, DocumentStatus};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::{DeliveryItem, DeliveryRequest, Subject};

use support::{logistics_booted, person};

/// The vehicle (an asset) of the run.
const VAN: i64 = 7;

fn day(days: i64) -> NaiveDate {
    if days >= 0 {
        today() + Days::new(days.unsigned_abs())
    } else {
        today() - Days::new(days.unsigned_abs())
    }
}

/// The accepted document of `kind` of `subject`, valid through `expires`.
async fn document(app: &TestApp, subject: &Subject, kind: &str, expires: NaiveDate) {
    let kind = DocumentKind::by_key(app.db(), kind)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {kind} is seeded"));
    Document::create(
        app.db(),
        Document {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            kind_id: kind.id,
            title: kind.name_en.clone(),
            expiry_date: Some(expires),
            current_version: 1,
            status: DocumentStatus::Accepted,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

fn van() -> Subject {
    Subject::new(defaults::VEHICLE_SUBJECT, VAN)
}

/// A planned run of `driver` today in a van.
async fn planned_run(app: &TestApp, driver: &User) -> Run {
    let mut run = Run {
        number: "RUN-T-1".to_owned(),
        run_date: today(),
        depot: "Depot A".to_owned(),
        driver_user_id: driver.id,
        vehicle_type: VehicleType::Van,
        vehicle_asset_id: Some(VAN),
        ..Run::default()
    };
    run.insert(app.db()).await.unwrap();
    run
}

/// A delivery of AED 100 to collect, from a test order.
async fn delivery(app: &TestApp, order: i64) -> Delivery {
    let request = DeliveryRequest::new(Subject::new("sales.order", order), "Mariam Al Hosani")
        .address("Villa 12, Al Barsha 2", "Al Barsha")
        .phone("+971500000000")
        .collect(10_000)
        .item(DeliveryItem::new("Parcel", 1_000));
    Deliveries::request(app.state(), request)
        .await
        .expect("the delivery is requested")
}

/// The stop form of the driver: the one line delivered in full, `cash`
/// collected and the difference `reason` when given.
async fn stop(
    app: &TestApp,
    driver: &User,
    delivery: &Delivery,
    cash: &str,
    reason: &str,
) -> renox::testing::TestResponse {
    let line = delivery.lines(app.db()).await.unwrap().remove(0);
    let delivered = format!("delivered_{}", line.id);
    let refused = format!("refused_{}", line.id);
    let mut fields: Vec<(&str, &str)> = vec![
        ("recipient_name", "Mariam Al Hosani"),
        ("photo", "photo-0001"),
        ("signature", "signature-0001"),
        ("latitude", "25.204849"),
        ("longitude", "55.270783"),
        ("cash_collected", cash),
        (delivered.as_str(), "1"),
        (refused.as_str(), "0"),
    ];
    if !reason.is_empty() {
        fields.push(("difference_reason", reason));
    }
    app.acting_as(driver);
    app.htmx()
        .post(&format!("/driver/stops/{}/deliver", delivery.id), &fields)
        .await
}

/// The driver and the dispatcher of a van run with valid documents, the run
/// planned and the delivery assigned to it.
async fn planned(app: &TestApp, order: i64) -> (User, User, Run, Delivery) {
    let driver = person(app, "Dev", DRIVER).await;
    let dispatcher = person(app, "Dina", DISPATCHER).await;
    document(
        app,
        &Subject::new(defaults::DRIVER_USER_SUBJECT, driver.id),
        "driving_licence",
        day(200),
    )
    .await;
    document(app, &van(), "vehicle_insurance", day(200)).await;
    document(app, &van(), "vehicle_permit", day(200)).await;
    let run = planned_run(app, &driver).await;
    let delivery = delivery(app, order).await;
    app.acting_as(&dispatcher);
    app.htmx()
        .post(
            &format!("/deliveries/{}/assign", delivery.id),
            &[("run_id", run.id.to_string().as_str())],
        )
        .await
        .assert_ok();
    (driver, dispatcher, run, delivery)
}

/// #323 AC1: the dispatch route refuses a run whose van has an expired
/// Mulkiya, with no override on offer; renewed, the same route dispatches it.
#[renox::test]
async fn the_dispatch_route_refuses_an_expired_mulkiya_with_no_override() {
    let app = logistics_booted().await;
    let (_driver, dispatcher, run, _delivery) = planned(&app, 1).await;
    document(&app, &van(), "mulkiya", day(-1)).await;

    app.acting_as(&dispatcher);
    let response = app
        .htmx()
        .post(&format!("/runs/{}/dispatch", run.id), &[])
        .await;
    let refused = response.assert_status(422).assert_see("Mulkiya");
    assert!(
        !refused.text().contains("/documents/overrides/new"),
        "a legal block offers no override"
    );
    assert_eq!(
        Run::find_or_404(app.db(), run.id).await.unwrap().status,
        RunStatus::Planned
    );

    document(&app, &van(), "mulkiya", day(300)).await;
    app.htmx()
        .post(&format!("/runs/{}/dispatch", run.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        Run::find_or_404(app.db(), run.id).await.unwrap().status,
        RunStatus::Dispatched
    );
}

/// #324 AC2: COD AED 100.00 collected AED 99.99 (tolerance 0) needs the
/// operations manager; the approver may pick the client, the business or the
/// driver and not the customer; the delivery is then delivered.
#[renox::test]
async fn a_cod_one_fil_short_needs_the_operations_manager_who_may_not_pick_the_customer() {
    let app = logistics_booted().await;
    let (driver, dispatcher, run, delivery) = planned(&app, 2).await;
    document(&app, &van(), "mulkiya", day(300)).await;
    app.acting_as(&dispatcher);
    app.htmx()
        .post(&format!("/runs/{}/dispatch", run.id), &[])
        .await
        .assert_ok();

    // Short by 0.01 without a reason is refused; with a reason it asks.
    stop(&app, &driver, &delivery, "99.99", "")
        .await
        .assert_status(422);
    stop(&app, &driver, &delivery, "99.99", "recipient_short")
        .await
        .assert_status(303);
    let request = ApprovalRequest::pending_about(app.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("AED 0.01 short asks for an approval");
    assert_eq!(request.action, cod::ACTION);
    assert_eq!(
        Delivery::find_or_404(app.db(), delivery.id)
            .await
            .unwrap()
            .status,
        DeliveryStatus::OutForDelivery
    );

    // The dispatcher may not approve; the operations manager may.
    let boss = person(&app, "Mona", OPERATIONS_MANAGER).await;
    let approve = format!("/deliveries/{}/cod-difference/approve", delivery.id);
    app.acting_as(&dispatcher);
    app.htmx()
        .post(&approve, &[("absorbed_by", "client")])
        .await
        .assert_forbidden();
    app.acting_as(&boss);
    app.htmx()
        .post(&approve, &[("absorbed_by", "customer")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&approve, &[("absorbed_by", "client")])
        .await
        .assert_ok();
    assert_eq!(
        Delivery::find_or_404(app.db(), delivery.id)
            .await
            .unwrap()
            .status,
        DeliveryStatus::Delivered
    );
}

/// #324 AC2: an exact collection asks nobody; a request nobody answers in
/// ten minutes lapses and the driver can only fail the attempt.
#[renox::test]
async fn an_unanswered_difference_lapses_after_ten_minutes_and_leaves_the_failed_attempt() {
    let app = logistics_booted().await;
    let (driver, dispatcher, run, delivery) = planned(&app, 3).await;
    document(&app, &van(), "mulkiya", day(300)).await;
    app.acting_as(&dispatcher);
    app.htmx()
        .post(&format!("/runs/{}/dispatch", run.id), &[])
        .await
        .assert_ok();

    stop(&app, &driver, &delivery, "99.99", "recipient_short")
        .await
        .assert_status(303);
    let request = ApprovalRequest::pending_about(app.db(), &delivery.delivery_id().subject())
        .await
        .unwrap()
        .pop()
        .expect("a request waits");
    let minutes = (request.expires_at.expect("a lapse time") - renox::db::now()).num_minutes();
    assert!((9..=10).contains(&minutes), "{minutes} minutes");

    app.travel(std::time::Duration::from_secs(11 * 60));
    let lapsed = app
        .at_travelled_time(cod::lapse_all(app.state()))
        .await
        .unwrap();
    assert_eq!(lapsed, 1);
    let after = ApprovalRequest::find(app.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Expired);
    // Only the failed attempt is left.
    stop(&app, &driver, &delivery, "99.99", "recipient_short")
        .await
        .assert_status(422);
    app.acting_as(&driver);
    app.htmx()
        .post(
            &format!("/driver/stops/{}/fail", delivery.id),
            &[
                ("reason", "refused_to_pay"),
                ("latitude", "25.204849"),
                ("longitude", "55.270783"),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(
        Delivery::find_or_404(app.db(), delivery.id)
            .await
            .unwrap()
            .status,
        DeliveryStatus::Attempted
    );
}
