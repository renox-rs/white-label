//! BL-MPW-12 (#426), BL-MPW-08 (#422) for `resume`, `MPW.RUL.12`,
//! `MPW.ACL.07`: with the real wl-documents, wl-attendance and wl-parties in
//! the host of a manpower agency, a renewed permit that is accepted resumes
//! the deployment (the worker can check in again, the cover request is
//! cancelled), a manual `resume` is refused while a document fails, a document
//! that still fails keeps the deployment suspended and tells the account
//! manager, and a blocked client does not stop a `resume`.

mod manpower_support;
mod support;

use host::roles::{ADMIN, COMPLIANCE, SUPERVISOR};
use manpower::suspension::{DeploymentSuspension, SuspensionKind};
use manpower::{Deployment, DeploymentStatus, ManpowerRequest, RequestKind, RequestStatus};
use manpower_support::{
    Account, account, all_valid_except, boot, deployment, document, reload, worker,
};
use renox::prelude::*;
use renox::testing::TestApp;
use renox::upload::Upload;
use support::{day, messages_to, person};
use wl_attendance::{AttendanceRecord, WorkLocation};
use wl_documents::expiry;
use wl_documents::{Documents, NewDocument};
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

/// A renewed `kind` of `who`, valid to `until`, uploaded and accepted by the
/// reviewer `rana`.
async fn renew(
    app: &TestApp,
    rana: &User,
    who: EmployeeId,
    kind: &str,
    until: renox::chrono::NaiveDate,
) {
    let upload = Upload::new(
        "renewal.pdf",
        "application/pdf",
        b"%PDF-1.7 renewal".to_vec(),
    );
    let attached = Documents::attach(
        app.state(),
        None,
        NewDocument::new(Subject::new("hr.employee", who.0), kind, upload).expires_on(until),
    )
    .await
    .unwrap()
    .expect("attached");
    app.acting_as(rana);
    app.htmx()
        .post(&format!("/documents/{}/accept", attached.id), &[])
        .await
        .assert_ok();
}

/// A suspended deployment as a fixture: suspended since `since` for `reasons`
/// (`(kind, reason)`), without the follow-up (nothing was announced).
async fn suspended(
    app: &TestApp,
    a: &Account,
    who: EmployeeId,
    since: renox::chrono::NaiveDate,
    reasons: &[(SuspensionKind, &str)],
) -> Deployment {
    let d = deployment(
        app,
        a,
        &a.site,
        who,
        "security_guard",
        DeploymentStatus::Suspended,
        day(-60),
        Some(day(200)),
    )
    .await;
    Deployment::query()
        .where_(Deployment::ID.eq(d.id))
        .update(app.db(), &[("suspended_on", &Some(since))])
        .await
        .unwrap();
    for (kind, reason) in reasons {
        DeploymentSuspension::create(
            app.db(),
            DeploymentSuspension {
                deployment_id: d.id,
                kind: *kind,
                reason: (*reason).to_owned(),
                expired_on: (*kind == SuspensionKind::Document)
                    .then(|| since - renox::chrono::Days::new(1)),
                added_on: since,
                ..DeploymentSuspension::default()
            },
        )
        .await
        .unwrap();
    }
    reload(app, d.id).await
}

/// AC1 and AC5 through the real chain: the nightly run suspends D for the
/// expired work permit and opens the cover request; the renewed permit is
/// accepted, D is `active`, the worker checks in at the site again, the client
/// is told, and the open cover request is cancelled with *worker returned*.
#[renox::test]
async fn the_renewed_permit_resumes_the_deployment_through_the_real_chain() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let supervisor = person(&app, "Sami", SUPERVISOR).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["work_permit"]).await;
    document(&app, w, "work_permit", day(-1)).await;
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
    let location = WorkLocation::create(
        app.db(),
        WorkLocation::named("Tower A").in_scope(&Subject::new("manpower.site", a.site.id)),
    )
    .await
    .unwrap();
    expiry::run(app.state(), day(0)).await.unwrap();
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Suspended);
    let cover = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .first(app.db())
        .await
        .unwrap()
        .expect("cover");
    assert_eq!(cover.status, RequestStatus::Open);

    renew(&app, &rana, w, "work_permit", day(700)).await;

    let back = reload(&app, d.id).await;
    assert_eq!(back.status, DeploymentStatus::Active);
    assert_eq!(back.resumed_on, Some(day(0)));
    assert_eq!(
        ManpowerRequest::find_or_404(app.db(), cover.id)
            .await
            .unwrap()
            .status,
        RequestStatus::Cancelled
    );
    let told = support::messages_to_party(&app, "manpower.client_notice", a.client).await;
    assert_eq!(told.len(), 2, "unavailable, then available again");
    assert!(
        told[1].data.0.to_string().contains("available again"),
        "{}",
        told[1].data.0
    );

    // W can check in at the site again.
    let employee = w.0.to_string();
    let place = location.id.to_string();
    let check_in = format!("{}T07:00", day(0));
    app.acting_as(&supervisor);
    app.htmx()
        .post(
            "/attendance/mark",
            &[
                ("employee_id", employee.as_str()),
                ("location_id", place.as_str()),
                ("check_in", check_in.as_str()),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(
        AttendanceRecord::query()
            .where_(AttendanceRecord::EMPLOYEE_ID.eq(w.0))
            .get(app.db())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// #422 AC6: a manual `resume` with an expired work permit is refused naming
/// the permit and the deployment stays `suspended`; once a permit valid through
/// the look-ahead window is accepted, the reason *site access withdrawn*
/// (a manual one, which no acceptance resumes) is resumed by the account manager.
#[renox::test]
async fn a_manual_resume_is_refused_while_the_work_permit_is_expired() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["work_permit"]).await;
    document(&app, w, "work_permit", day(-5)).await;
    let d = suspended(
        &app,
        &a,
        w,
        day(-3),
        &[(SuspensionKind::Manual, "site access withdrawn")],
    )
    .await;
    let path = format!("/manpower/deployments/{}/resume", d.id);

    let refused = manpower_support::post(&app, &a.amal, &path, &[]).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Work permit"), "{body}");
    assert!(body.contains("override: not allowed"), "{body}");
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Suspended);

    // A permit that ends inside the 30 days still fails; one beyond passes.
    renew(&app, &rana, w, "work_permit", day(10)).await;
    assert_eq!(
        reload(&app, d.id).await.status,
        DeploymentStatus::Suspended,
        "a manual reason is not resumed by a document"
    );
    manpower_support::post(&app, &a.amal, &path, &[])
        .await
        .assert_status(422);
    renew(&app, &rana, w, "work_permit", day(700)).await;
    manpower_support::post(&app, &a.amal, &path, &[])
        .await
        .assert_status(303);
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
}

/// The renewed permit is accepted, but the residence visa expires in 10 days
/// (inside the look-ahead): D stays `suspended`, the reason of the permit is
/// gone, and the account manager is told which kind still fails.
#[renox::test]
async fn a_kind_that_still_fails_keeps_the_deployment_suspended_and_tells_the_manager() {
    let app = boot().await;
    let a = account(&app).await;
    let rana = person(&app, "Rana", ADMIN).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["work_permit", "residence_visa"]).await;
    document(&app, w, "work_permit", day(-4)).await;
    document(&app, w, "residence_visa", day(10)).await;
    let d = suspended(
        &app,
        &a,
        w,
        day(-3),
        &[(SuspensionKind::Document, "work_permit")],
    )
    .await;

    renew(&app, &rana, w, "work_permit", day(700)).await;

    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Suspended);
    assert!(
        DeploymentSuspension::open_for(app.db(), d.id)
            .await
            .unwrap()
            .is_empty(),
        "the reason of the accepted kind is removed"
    );
    let told = messages_to(&app, "manpower.notice", &a.amal).await;
    assert!(
        told.iter().any(|m| {
            let text = m.data.0.to_string();
            text.contains("stays suspended") && text.contains("Residence visa")
        }),
        "{told:?}"
    );

    // The visa is renewed: the next acceptance resumes it.
    renew(&app, &rana, w, "residence_visa", day(700)).await;
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
}

/// A blocked client company (`continue`) does not stop a `resume`: the worker
/// was on site before; staff without the permission to see the reason are
/// refused `activate` but not `resume`.
#[renox::test]
async fn a_blocked_client_does_not_stop_a_resume() {
    let app = boot().await;
    let a = account(&app).await;
    let officer = person(&app, "Comply", COMPLIANCE).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &[]).await;
    let d = suspended(
        &app,
        &a,
        w,
        day(-3),
        &[(SuspensionKind::Manual, "investigation")],
    )
    .await;
    app.acting_as(&officer);
    app.htmx()
        .post(
            &format!("/parties/{}/block", a.client.0),
            &[
                ("reason_code", "fraud_suspected"),
                ("note", "Forged cheques"),
            ],
        )
        .await
        .assert_ok();
    let planned = deployment(
        &app,
        &a,
        &a.site,
        worker(&app, "Omar Said").await,
        "cleaner",
        DeploymentStatus::Planned,
        day(0),
        Some(day(100)),
    )
    .await;
    manpower_support::activate(&app, &a.amal, &planned)
        .await
        .assert_status(422);

    manpower_support::post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/resume", d.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
}
