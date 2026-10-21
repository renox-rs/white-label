//! BL-MPW-08 (#422), `MPW.ACL.07`, `DOC.ACL.02`: with the real wl-documents,
//! wl-hr and wl-notify in the host of a manpower agency, a deployment's
//! `activate` and `extend` are refused unless the worker's residence visa,
//! work permit, Emirates ID and health insurance and the licences of the role
//! are valid on the day of the action plus 30 days (or the end date, if
//! earlier); an override is offered only as the kind allows, once, with the
//! MD's approval; planning a deployment only warns. `resume` is tested with
//! its transition (BL-MPW-12).

mod manpower_support;
mod support;

use host::roles::ADMIN;
use manpower::DeploymentStatus;
use manpower_support::{
    account, activate, all_valid_except, boot, boot_with, deployment, document, extend, post,
    reload, worker,
};
use renox::prelude::*;
use support::{day, person};
use wl_documents::{BlockOverrideStatus, Documents};
use wl_kernel::approvals;
use wl_kernel::documents::OverrideMode;

/// AC1: W's residence visa expires in 20 days; D (a security guard, running
/// 200 days) is refused `activate` because the visa must be valid 30 days on,
/// the refusal names the visa and its expiry date and offers no override.
#[renox::test]
async fn a_visa_that_expires_inside_the_look_ahead_refuses_activate_with_no_override() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["residence_visa"]).await;
    document(&app, w, "residence_visa", day(20)).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;

    let refused = activate(&app, &a.amal, &d).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Residence visa"), "{body}");
    assert!(body.contains(&day(20).to_string()), "{body}");
    assert!(body.contains("override: not allowed"), "{body}");
    assert!(
        body.contains(&format!("judged on {}", day(30))),
        "the day judged is named: {body}"
    );
    assert!(
        !body.contains("overrides/new"),
        "no override offered: {body}"
    );
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Planned);
}

/// AC2: the same visa and D ending in 10 days: the documents are needed to the
/// end date only, so `activate` is allowed.
#[renox::test]
async fn a_deployment_shorter_than_the_look_ahead_needs_its_documents_to_its_end_only() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["residence_visa"]).await;
    document(&app, w, "residence_visa", day(20)).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(10)),
    )
    .await;

    activate(&app, &a.amal, &d).await.assert_status(303);
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
}

/// AC3: W's health insurance expired and all else is valid: `activate` is
/// refused offering an override; account manager A asks, MD B approves, A
/// activates; a second `activate` of another deployment of W needs a new
/// override.
#[renox::test]
async fn an_expired_insurance_is_overridden_once_with_the_mds_approval() {
    let app = boot().await;
    let a = account(&app).await;
    let md = person(&app, "Rana", ADMIN).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["health_insurance"]).await;
    document(&app, w, "health_insurance", day(-3)).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;
    let mut other = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;
    other.shift_start = Some("16:00".into());
    other.shift_end = Some("23:00".into());
    manpower::Deployment::query()
        .where_(manpower::Deployment::ID.eq(other.id))
        .update(
            app.db(),
            &[
                ("shift_start", &other.shift_start),
                ("shift_end", &other.shift_end),
            ],
        )
        .await
        .unwrap();

    let refused = activate(&app, &a.amal, &d).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Health insurance"), "{body}");
    assert!(body.contains("override: with approval"), "{body}");

    let rows = Documents::request_override(
        app.state(),
        &a.amal,
        "health_insurance",
        "deployment.activate",
        d.id,
        "renewal booked",
    )
    .await
    .unwrap();
    let request = rows[0].approval_request_id.expect("an approval request");
    approvals::approve(app.state(), request, &md, "")
        .await
        .expect("the MD approves");

    activate(&app, &a.amal, &d).await.assert_status(303);
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
    let used = wl_documents::BlockOverride::find(app.db(), rows[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.status, BlockOverrideStatus::Used);

    // One use: the same override does not cover another deployment.
    let second = activate(&app, &a.amal, &other).await;
    second.assert_status(422);
    assert!(
        second.text().contains("Health insurance"),
        "{}",
        second.text()
    );
    assert_eq!(
        reload(&app, other.id).await.status,
        DeploymentStatus::Planned
    );
}

/// AC4: the security role needs the health card of its site; W has none, so
/// `activate` is refused; a deployment in a role the card is not required for
/// goes through.
#[renox::test]
async fn a_licence_the_role_needs_refuses_activate_and_another_role_does_not_need_it() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    for kind in [
        "residence_visa",
        "work_permit",
        "emirates_id",
        "health_insurance",
    ] {
        document(&app, w, kind, day(365)).await;
    }
    let guard = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;
    let cleaner = deployment(
        &app,
        &a,
        &a.site,
        w,
        "cleaner",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;

    let refused = activate(&app, &a.amal, &guard).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Health card"), "{body}");
    assert!(body.contains("SIRA licence"), "{body}");
    activate(&app, &a.amal, &cleaner).await.assert_status(303);
}

/// AC5: `extend` of the active D2 to 150 days ahead with W's Emirates ID
/// expiring in 20 days is refused; an extension that ends before the card
/// expires is allowed.
#[renox::test]
async fn extend_is_judged_on_the_new_end_date() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["emirates_id"]).await;
    document(&app, w, "emirates_id", day(20)).await;
    let d2 = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-30),
        Some(day(5)),
    )
    .await;

    let refused = extend(&app, &a.amal, &d2, day(150)).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("Emirates ID"), "{body}");
    assert_eq!(reload(&app, d2.id).await.end_date, Some(day(5)));

    extend(&app, &a.amal, &d2, day(15)).await.assert_status(303);
    assert_eq!(reload(&app, d2.id).await.end_date, Some(day(15)));
}

/// AC7: planning D7 (starting in 40 days) for W, whose visa expires in 10 days,
/// saves it and shows a warning; its `activate` is refused later.
#[renox::test]
async fn planning_only_warns_and_activate_is_refused_later() {
    let app = boot().await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["residence_visa"]).await;
    document(&app, w, "residence_visa", day(10)).await;

    let employee = w.0.to_string();
    let contract = a.contract.id.to_string();
    let site = a.site.id.to_string();
    let start = day(40).to_string();
    let end = day(100).to_string();
    // A plain POST, so the toast waits in the session.
    app.acting_as(&a.amal);
    app.post(
        "/manpower/deployments",
        &[
            ("employee_id", employee.as_str()),
            ("client_contract_id", contract.as_str()),
            ("site_id", site.as_str()),
            ("role", "security_guard"),
            ("shift_type", "day"),
            ("start_date", start.as_str()),
            ("end_date", end.as_str()),
        ],
    )
    .await
    .assert_status(303);
    let toasts = app
        .session_get::<renox::serde_json::Value>("_toasts")
        .map(|toasts| toasts.to_string())
        .unwrap_or_default();
    assert!(toasts.contains("Residence visa"), "toasts: [{toasts}]");
    assert!(toasts.contains(&day(40).to_string()), "{toasts}");
    let planned = manpower::Deployment::query()
        .where_(manpower::Deployment::EMPLOYEE_ID.eq(w.0))
        .first(app.db())
        .await
        .unwrap()
        .expect("D7 is saved");
    assert_eq!(planned.status, DeploymentStatus::Planned);

    // Later, on its start date, the visa still has not been renewed.
    let late = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(100)),
    )
    .await;
    activate(&app, &a.amal, &late).await.assert_status(422);
}

/// AC8: with the Emirates ID override mode set to `never` in the seed, the
/// health insurance case still offers the override and the Emirates ID case
/// offers none: the mode is read from the kind, with no change of code.
#[renox::test]
async fn the_override_mode_is_read_from_the_kind_setting() {
    let app = boot_with(OverrideMode::Never).await;
    let a = account(&app).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid_except(&app, w, &["health_insurance", "emirates_id"]).await;
    document(&app, w, "health_insurance", day(-3)).await;
    document(&app, w, "emirates_id", day(-3)).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(0),
        Some(day(200)),
    )
    .await;

    let refused = activate(&app, &a.amal, &d).await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(
        body.contains("Health insurance expired") && body.contains("override: with approval"),
        "{body}"
    );
    assert!(
        body.contains("Emirates ID expired") && body.contains("renew: hr; override: not allowed"),
        "{body}"
    );
    assert!(
        Documents::request_override(
            app.state(),
            &a.amal,
            "emirates_id",
            "deployment.activate",
            d.id,
            "renewal booked",
        )
        .await
        .is_err()
    );
    assert!(
        Documents::request_override(
            app.state(),
            &a.amal,
            "health_insurance",
            "deployment.activate",
            d.id,
            "renewal booked",
        )
        .await
        .is_ok()
    );
}
