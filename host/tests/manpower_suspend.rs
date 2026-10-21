//! BL-MPW-11 (#425), `MPW.RUL.12` (the expiry part), `DOC.RUL.03`, `MPW.DEP.02`:
//! through the real wl-documents, wl-attendance and wl-notify in the host of a
//! manpower agency, the nightly run reports the expired work permit and the
//! deployment is `suspended` the day after: the worker's check-in at the site
//! is refused, the client and the account manager get their messages, the
//! cover request is open, a further kind adds a reason without a second message
//! and a planned deployment is left alone.

mod manpower_support;
mod support;

use host::roles::SUPERVISOR;
use manpower::suspension::DeploymentSuspension;
use manpower::{DeploymentStatus, ManpowerRequest, RequestKind, RequestStatus};
use manpower_support::{account, all_valid_except, boot, deployment, document, reload, worker};
use renox::prelude::*;
use support::{day, messages_to, messages_to_party, person};
use wl_attendance::{AttendanceRecord, WorkLocation};
use wl_documents::expiry;
use wl_kernel::Subject;

/// AC1, AC3, AC4 and the refused check-in: W's work permit and health
/// insurance expired yesterday; the nightly run suspends D (active) today, D8
/// (planned) is left alone, the second kind adds a reason and no second
/// message.
#[renox::test]
async fn the_nightly_run_suspends_the_deployment_and_the_check_in_is_refused() {
    let app = boot().await;
    let a = account(&app).await;
    let supervisor = person(&app, "Sami", SUPERVISOR).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["work_permit", "health_insurance"]).await;
    document(&app, w, "work_permit", day(-1)).await;
    document(&app, w, "health_insurance", day(-1)).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-60),
        Some(day(200)),
    )
    .await;
    let other = manpower_support::site(&app, a.client, "Tower B").await;
    let d8 = deployment(
        &app,
        &a,
        &other,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(30),
        Some(day(100)),
    )
    .await;
    let location = WorkLocation::create(
        app.db(),
        WorkLocation::named("Tower A").in_scope(&Subject::new("manpower.site", a.site.id)),
    )
    .await
    .unwrap();

    expiry::run(app.state(), day(0)).await.unwrap();

    let suspended = reload(&app, d.id).await;
    assert_eq!(suspended.status, DeploymentStatus::Suspended);
    assert_eq!(suspended.suspended_on, Some(day(0)));
    assert_eq!(reload(&app, d8.id).await.status, DeploymentStatus::Planned);

    // The worker cannot check in at the site; the supervisor's mark fails the
    // same way and nothing is stored.
    let employee = w.0.to_string();
    let place = location.id.to_string();
    let check_in = format!("{}T07:00", day(0));
    app.acting_as(&supervisor);
    let refused = app
        .htmx()
        .post(
            "/attendance/mark",
            &[
                ("employee_id", employee.as_str()),
                ("location_id", place.as_str()),
                ("check_in", check_in.as_str()),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("suspended"), "{}", refused.text());
    assert!(
        AttendanceRecord::query()
            .where_(AttendanceRecord::EMPLOYEE_ID.eq(w.0))
            .get(app.db())
            .await
            .unwrap()
            .is_empty()
    );

    // The client: neutral. The account manager: the kind.
    let client = messages_to_party(&app, "manpower.client_notice", a.client).await;
    assert_eq!(client.len(), 1);
    let told = client[0].data.0.to_string();
    assert!(told.contains("temporarily unavailable"), "{told}");
    assert!(!told.to_lowercase().contains("permit"), "{told}");
    let staff = messages_to(&app, "manpower.notice", &a.amal).await;
    assert!(
        staff
            .iter()
            .any(|m| m.data.0.to_string().contains("work permit")),
        "the account manager is told"
    );

    // The replacement request: one worker, same role and site, open from today.
    let covers = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    assert_eq!(covers[0].start_date, day(0));
    assert_eq!(covers[0].end_date, None);
    assert_eq!(covers[0].headcount, 1);
    assert_eq!(covers[0].role, "security_guard");
    assert_eq!(covers[0].site_id, a.site.id);

    // The insurance expired yesterday, too: a second reason, and the client
    // is told once.
    let reasons = DeploymentSuspension::open_for(app.db(), d.id)
        .await
        .unwrap();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert_eq!(
        messages_to_party(&app, "manpower.client_notice", a.client)
            .await
            .len(),
        1
    );
}
