//! BL-MPW-13 (#427), `MPW.RUL.12` (the exit part), `HRM.ACL.07`: with the real
//! wl-hr in the host of a manpower agency, an approved `record_death` ends the
//! worker's active deployment and cancels the planned one, an approved
//! `cancel_onboarding` cancels the planned deployment, and the client is told
//! only that the worker is no longer available; an approved `withdraw_notice`
//! restores the end dates and a rejected renewal tells the staff.

mod manpower_support;
mod support;

use host::roles::{HR, OWNER};
use manpower::DeploymentStatus;
use manpower_support::{account, boot, deployment, reload};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{day, messages_to, messages_to_party, person};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;

/// HR asks for `fields` of `transition` on the employee and the MD approves.
async fn exit(
    app: &renox::testing::TestApp,
    who: EmployeeId,
    transition: &str,
    fields: &[(&str, &str)],
) {
    let hr = person(app, "Hana", HR).await;
    let md = person(app, "Basel", OWNER).await;
    let path = format!("/hr/employees/{}/exits", who.0);
    let mut form = vec![("transition", transition)];
    form.extend_from_slice(fields);
    app.acting_as(&hr);
    app.htmx().post(&path, &form).await.assert_ok();
    let request = ApprovalRequest::pending_about(app.db(), &Subject::new("hr.employee", who.0))
        .await
        .unwrap()
        .remove(0);
    approvals::approve(app.state(), request.id, &md, "")
        .await
        .unwrap();
}

/// AC1 and AC2 through the real wl-hr: W has an active D1 and a planned D2;
/// HR records the death of W yesterday and the MD approves: D1 is `ended` with
/// the reason *deceased* and yesterday as its last day, D2 is `cancelled`, the
/// client is told W is no longer available (never that W died) and the account
/// manager is told.
#[renox::test]
async fn an_approved_death_ends_and_cancels_the_deployments_of_the_worker() {
    let app = boot().await;
    let a = account(&app).await;
    let w = manpower_support::worker(&app, "Wasim Khan").await;
    let d1 = deployment(
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
    let d2 = deployment(
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

    let died = day(-1).to_string();
    exit(
        &app,
        w,
        "record_death",
        &[
            ("reason", "died in service"),
            ("death_certificate", "DC-9"),
            ("death_date", died.as_str()),
        ],
    )
    .await;

    let ended = reload(&app, d1.id).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("deceased"));
    assert_eq!(ended.end_date, Some(day(-1)));
    assert_eq!(
        reload(&app, d2.id).await.status,
        DeploymentStatus::Cancelled
    );

    let client = messages_to_party(&app, "manpower.client_notice", a.client).await;
    assert_eq!(client.len(), 1, "D2 filled no request of the client");
    let told = client[0].data.0.to_string();
    assert!(told.contains("no longer available"), "{told}");
    assert!(
        !told.to_lowercase().contains("died") && !told.to_lowercase().contains("death"),
        "{told}"
    );
    let staff = messages_to(&app, "manpower.notice", &a.amal).await;
    assert!(
        staff
            .iter()
            .any(|m| m.data.0.to_string().contains("worker deceased")),
        "the account manager is told"
    );
}

/// AC3 through the real wl-hr: W is still in `onboarding` with a planned D3;
/// the approved `cancel_onboarding` cancels D3.
#[renox::test]
async fn an_approved_cancel_onboarding_cancels_the_planned_deployment() {
    let app = boot().await;
    let a = account(&app).await;
    let party = support::tenant(&app, "Omar Said").await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Onboarding),
    )
    .await
    .unwrap();
    let w = EmployeeId(employee.id);
    let d3 = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(10),
        Some(day(100)),
    )
    .await;

    exit(
        &app,
        w,
        "cancel_onboarding",
        &[("reason", "the candidate withdrew")],
    )
    .await;

    let cancelled = reload(&app, d3.id).await;
    assert_eq!(cancelled.status, DeploymentStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("onboarding cancelled")
    );
}

/// AC4 through the real wl-hr: W serves notice with a last working day 30 days
/// ahead (`give_notice` is not built in wl-hr, so the event it will emit is
/// emitted as it will be); D4 had an end date 200 days ahead, D4' none. HR asks
/// for `withdraw_notice` with the written agreement and the MD approves: wl-hr
/// returns W to `active` and emits the event itself, and the end dates are back.
#[renox::test]
async fn an_approved_withdrawal_of_the_notice_restores_the_end_dates() {
    use wl_kernel::workforce::EmployeeNoticeGiven;

    let app = boot().await;
    let a = account(&app).await;
    let party = support::tenant(&app, "Wasim Khan").await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    let w = EmployeeId(employee.id);
    // `give_notice` is not built: W is in `notice` as it will leave W.
    renox::db::sql(
        "UPDATE wl_hr_employees SET status = 'notice', last_working_day = ? WHERE id = ?",
    )
    .bind(day(30))
    .bind(w.0)
    .execute(app.db())
    .await
    .unwrap();
    let d4 = deployment(
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
    let d4b = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Planned,
        day(10),
        None,
    )
    .await;
    app.state()
        .emit(EmployeeNoticeGiven {
            employee: w,
            last_working_day: day(30),
        })
        .await
        .unwrap();
    assert_eq!(reload(&app, d4.id).await.end_date, Some(day(30)));
    assert_eq!(reload(&app, d4b.id).await.end_date, Some(day(30)));

    exit(
        &app,
        w,
        "withdraw_notice",
        &[("reason", "both sides agreed"), ("agreement", "AGR-1")],
    )
    .await;

    let back = wl_hr::Employee::find_or_404(app.db(), w.0).await.unwrap();
    assert_eq!(back.status, EmployeeStatus::Active);
    assert_eq!(reload(&app, d4.id).await.end_date, Some(day(200)));
    assert_eq!(reload(&app, d4b.id).await.end_date, None);
    assert_eq!(reload(&app, d4.id).await.status, DeploymentStatus::Active);
    let staff = messages_to(&app, "manpower.notice", &a.amal).await;
    assert_eq!(
        staff.len(),
        1,
        "the account manager is told once: {staff:?}"
    );
}

/// AC6 through the real wl-hr: W's renewal case is `in_progress` and the visa
/// expires 100 days ahead; D5 runs 200 days ahead, D6 ends in 50. The PRO
/// rejects the case on the page: wl-hr emits `VisaCaseRejected`, the account
/// manager and the MD are told about D5 only, D5 stays `active` and the client
/// is told nothing.
#[renox::test]
async fn a_rejected_renewal_tells_the_staff_about_the_deployments_past_the_visa() {
    use host::roles::{ADMIN, PRO};

    let app = boot().await;
    let a = account(&app).await;
    let pro = person(&app, "Pia", PRO).await;
    // The MD is the first company admin (`role_tasks::assignee`).
    let md = renox::auth::permissions::users_with_role(app.db(), ADMIN)
        .await
        .unwrap()
        .remove(0);
    let w = manpower_support::worker(&app, "Wasim Khan").await;
    let d5 = deployment(
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
    let d6 = deployment(
        &app,
        &a,
        &other,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-60),
        Some(day(50)),
    )
    .await;
    let mut case = wl_hr::VisaCase {
        employee_id: w.0,
        case_type: "renewal".into(),
        variant: "standard".into(),
        status: wl_hr::VisaCaseStatus::InProgress,
        current_expiry: Some(day(100)),
        ..wl_hr::VisaCase::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    case.insert_in_state(&mut tx, wl_hr::VisaCaseStatus::InProgress, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    app.acting_as(&pro);
    app.post(
        &format!("/hr/visa-cases/{}/reject", case.id),
        &[
            ("reference", "GDRFA-123"),
            ("reason", "Passport copy unreadable"),
        ],
    )
    .await
    .assert_status(303);

    for who in [&a.amal, &md] {
        let told = messages_to(&app, "manpower.notice", who).await;
        assert_eq!(told.len(), 1, "{}: {told:?}", who.name);
        let body = told[0].data.0.to_string();
        assert!(body.contains(&day(100).to_string()), "{body}");
        assert!(body.contains(&format!("ends {}", day(200))), "{body}");
        assert!(
            !body.contains(&day(50).to_string()),
            "D6 ends first: {body}"
        );
    }
    assert!(
        messages_to_party(&app, "manpower.client_notice", a.client)
            .await
            .is_empty(),
        "the client is told nothing"
    );
    assert_eq!(reload(&app, d5.id).await.status, DeploymentStatus::Active);
    assert_eq!(reload(&app, d6.id).await.status, DeploymentStatus::Active);
}
