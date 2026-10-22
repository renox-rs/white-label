//! BL-MPW-17 (#431), `MPW.RUL.09`, BL-HRM-07 and BL-HRM-18 with the real
//! wl-hr, wl-ticketing and wl-notify in the host of a manpower agency: the
//! daily run opens the absence case (a real ticket of the category `absence`
//! for HR, wl-hr's own case, the cover request, the client told); with one
//! failed attempt `report_absconding` is refused, with two and the MoHRE
//! number the MD approves it and the deployments end (BL-MPW-13); a worker who
//! returns has the case closed by HR and the held line comes back in a draft
//! off-cycle run.

mod manpower_support;
mod support;

use host::roles::{ADMIN, HR, OWNER};
use manpower::absence::{self, AbsenceCase};
use manpower::{DeploymentStatus, ManpowerRequest, RequestKind, RequestStatus};
use manpower_support::{
    account, all_valid, boot, deployment, draft_run, post, reload, run_transition, salary, worker,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{day, messages_to, messages_to_party, person};
use wl_hr::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_ticketing::{Category, Ticket};

/// HR records a failed attempt to reach `who` on `on` by `method`.
async fn attempt(
    app: &renox::testing::TestApp,
    hr: &User,
    who: i64,
    on: renox::chrono::NaiveDate,
    method: &str,
) {
    app.acting_as(hr);
    let on = on.to_string();
    app.htmx()
        .post(
            &format!("/hr/employees/{who}/contact-attempts"),
            &[
                ("attempted_on", on.as_str()),
                ("method", method),
                ("outcome", "no_answer"),
            ],
        )
        .await
        .assert_ok();
}

/// AC1, AC2 and AC4 with the real modules: W has been on site for 10 days
/// without attendance. The run opens the case; HR is reminded; with one failed
/// attempt `report_absconding` is refused; with two and the number "MOHRE-777"
/// the MD approves, W is `absconded` and the deployment is `ended` with that
/// reason, the cover request stays for the account manager.
#[renox::test]
async fn the_case_leads_to_the_report_and_the_end_of_the_deployment() {
    let app = boot().await;
    let a = account(&app).await;
    let hr = person(&app, "Hana", HR).await;
    let md = person(&app, "Basel", OWNER).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid(&app, w).await;
    let d = deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-10),
        Some(day(200)),
    )
    .await;

    absence::run_on(app.state(), today()).await.unwrap();

    let case = AbsenceCase::open_of(app.db(), w.0)
        .await
        .unwrap()
        .expect("the case");
    assert_eq!(case.last_present_on, day(-10));
    assert_eq!(reload(&app, d.id).await.status, DeploymentStatus::Active);
    // The real ticket: category absence, about W, assigned to HR.
    let category = Category::by_key(app.db(), "absence")
        .await
        .unwrap()
        .expect("the absence category is seeded");
    let tickets = Ticket::query()
        .where_(Ticket::CATEGORY_ID.eq(category.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject_type, "hr.employee");
    assert_eq!(tickets[0].subject_id, w.0);
    assert_eq!(tickets[0].assignee_user_id, Some(hr.id));
    assert_eq!(case.ticket_id, Some(tickets[0].id));
    // wl-hr's own case, which `report_absconding` and the pay hold need.
    let theirs = wl_hr::AbsenceCase::open_of(app.db(), w.0).await.unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0].last_present_on, day(-10));
    // The cover request, the client told, HR reminded.
    let covers = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(covers.len(), 1);
    assert_eq!(covers[0].status, RequestStatus::Open);
    let client = messages_to_party(&app, "manpower.client_notice", a.client).await;
    assert!(
        client
            .iter()
            .any(|m| m.data.0.to_string().contains("has not turned up")),
        "the client is told"
    );
    let reminders = messages_to(&app, "manpower.notice", &hr).await;
    assert!(
        reminders
            .iter()
            .any(|m| m.data.0.to_string().contains("Failed attempts to reach")),
        "HR is reminded"
    );

    // The MoHRE number is recorded on the case.
    app.acting_as(&hr);
    let on = today().to_string();
    app.htmx()
        .post(
            &format!("/manpower/absence/{}/report", case.id),
            &[("number", "MOHRE-777"), ("date", on.as_str())],
        )
        .await
        .assert_status(303);

    // One failed attempt: `report_absconding` is refused.
    let date = today().to_string();
    attempt(&app, &hr, w.0, day(-3), "phone").await;
    app.acting_as(&hr);
    let form = vec![
        ("transition", "report_absconding"),
        ("reason", "absent for 10 days"),
        ("report_number", "MOHRE-777"),
        ("report_date", date.as_str()),
    ];
    app.htmx()
        .post(&format!("/hr/employees/{}/exits", w.0), &form)
        .await
        .assert_status(422);

    // The reminders go on until both are recorded; then two attempts allow it.
    attempt(&app, &hr, w.0, day(-1), "sms").await;
    app.acting_as(&hr);
    app.htmx()
        .post(&format!("/hr/employees/{}/exits", w.0), &form)
        .await
        .assert_ok();
    let pending = ApprovalRequest::pending_about(app.db(), &Subject::new("hr.employee", w.0))
        .await
        .unwrap()
        .remove(0);
    approvals::approve(app.state(), pending.id, &md, "")
        .await
        .unwrap();

    let ended = reload(&app, d.id).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("absconded"));
    // The MD's approval opened the cancellation case from the absconding
    // variant of the step template (BL-MPW-21).
    let cases = wl_hr::VisaCase::cancellations_of(app.db(), w.0)
        .await
        .unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].variant, "absconding");
    assert_eq!(cases[0].report_number.as_deref(), Some("MOHRE-777"));
}

/// AC1 and AC5 with the real wl-hr: the case is open when HR prepares the
/// November run, so W's line is held ("absence case") by `calculate` itself;
/// the run is approved by a second user and paid, W's line stays held. W
/// returns, HR closes the case with "hospitalised": the ticket is completed,
/// the open cover request cancelled, and wl-hr proposes an off-cycle run in
/// `draft` with W's held line. The preparer of the off-cycle run cannot
/// approve it (`HRM.ACL.04`); it is paid after a second user approves.
#[renox::test]
async fn closing_the_case_releases_the_held_line_into_an_off_cycle_run_paid_after_a_second_approval()
 {
    let app = boot().await;
    let a = account(&app).await;
    let hr = person(&app, "Hana", HR).await;
    let md = person(&app, "Basel", OWNER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let w = worker(&app, "Wasim Khan").await;
    all_valid(&app, w).await;
    salary(&app, w, 300_000).await;
    deployment(
        &app,
        &a,
        &a.site,
        w,
        "security_guard",
        DeploymentStatus::Active,
        day(-10),
        Some(day(200)),
    )
    .await;

    absence::run_on(app.state(), today()).await.unwrap();
    let case = AbsenceCase::open_of(app.db(), w.0)
        .await
        .unwrap()
        .expect("the case");

    // AC1: the hold is what `calculate` makes of the open case.
    let november = draft_run(&app, "2026-11").await;
    run_transition(&app, &hr, &november, "calculate")
        .await
        .assert_status(303);
    let held = line_of(&app, &november, w.0).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.held_reason.as_deref(), Some("absence case"));
    run_transition(&app, &md, &november, "approve")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(app.state(), november.id)
        .await
        .unwrap();
    post(
        &app,
        &md,
        &format!("/hr/payroll/{}/confirm", november.id),
        &[("confirmation", "BANK-1")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        PayrollRun::find_or_404(app.db(), november.id)
            .await
            .unwrap()
            .status,
        PayrollRunStatus::Paid
    );
    assert_eq!(
        line_of(&app, &november, w.0).await.status,
        LineStatus::Held,
        "W is not paid while the case is open"
    );

    app.acting_as(&hr);
    app.htmx()
        .post(
            &format!("/manpower/absence/{}/close", case.id),
            &[("reason", "hospitalised")],
        )
        .await
        .assert_status(303);

    assert!(AbsenceCase::open_of(app.db(), w.0).await.unwrap().is_none());
    assert!(
        wl_hr::AbsenceCase::open_of(app.db(), w.0)
            .await
            .unwrap()
            .is_empty(),
        "wl-hr's case is closed too"
    );
    let off = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq("off_cycle"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].status, PayrollRunStatus::Draft);
    let lines = off[0].lines(app.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].employee_id, w.0);
    assert_eq!(lines[0].source_line_id, Some(held.id));
    let covers = ManpowerRequest::query()
        .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(covers[0].status, RequestStatus::Cancelled);

    // AC5: the admin prepares the off-cycle run and cannot approve it; the
    // MD, a second user, does, and only then W is paid.
    let off = off[0].clone();
    run_transition(&app, &admin, &off, "calculate")
        .await
        .assert_status(303);
    run_transition(&app, &admin, &off, "approve")
        .await
        .assert_status(422);
    assert_eq!(
        PayrollRun::find_or_404(app.db(), off.id)
            .await
            .unwrap()
            .status,
        PayrollRunStatus::Calculated
    );
    assert_ne!(line_of(&app, &off, w.0).await.status, LineStatus::Paid);
    run_transition(&app, &md, &off, "approve")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(app.state(), off.id).await.unwrap();
    post(
        &app,
        &md,
        &format!("/hr/payroll/{}/confirm", off.id),
        &[("confirmation", "BANK-2")],
    )
    .await
    .assert_status(303);
    assert_eq!(line_of(&app, &off, w.0).await.status, LineStatus::Paid);
    assert_eq!(
        PayrollLine::find_or_404(app.db(), held.id)
            .await
            .unwrap()
            .released_into,
        Some(off.id)
    );
}

/// The line of employee `employee` in `run`.
async fn line_of(app: &renox::testing::TestApp, run: &PayrollRun, employee: i64) -> PayrollLine {
    PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run.id))
        .where_(PayrollLine::EMPLOYEE_ID.eq(employee))
        .first(app.db())
        .await
        .unwrap()
        .expect("the line")
}

/// Approved leave counts as present, through the real wl-hr leave calendar:
/// W was on approved leave until 5 days ago, so ten days after the
/// deployment began there is no case; X, with the same deployment and no
/// leave, has one. A leave that ended 9 days ago leaves 9 days: a case, with
/// day 0 at the last day of leave.
#[renox::test]
async fn approved_leave_in_wl_hr_counts_as_present() {
    use wl_hr::{LeaveRequest, LeaveStatus};

    let app = boot().await;
    let a = account(&app).await;
    let on_leave = worker(&app, "Wasim Khan").await;
    let back = worker(&app, "Basim Khan").await;
    let nobody = worker(&app, "Xavier Khan").await;
    for who in [on_leave, back, nobody] {
        all_valid(&app, who).await;
        deployment(
            &app,
            &a,
            &a.site,
            who,
            "security_guard",
            DeploymentStatus::Active,
            day(-20),
            Some(day(200)),
        )
        .await;
    }
    for (who, from, to, status) in [
        (on_leave, day(-15), day(-5), LeaveStatus::Approved),
        (back, day(-15), day(-9), LeaveStatus::Completed),
        // Not approved: it counts for nothing.
        (nobody, day(-15), day(-5), LeaveStatus::Requested),
    ] {
        let mut request = LeaveRequest {
            employee_id: who.0,
            leave_type: "annual".into(),
            from_date: from,
            to_date: to,
            status,
            ..LeaveRequest::default()
        };
        let mut tx = app.db().begin().await.unwrap();
        request
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    absence::run_on(app.state(), today()).await.unwrap();

    assert!(
        AbsenceCase::open_of(app.db(), on_leave.0)
            .await
            .unwrap()
            .is_none(),
        "5 days after approved leave is not 7"
    );
    let case = AbsenceCase::open_of(app.db(), back.0)
        .await
        .unwrap()
        .expect("9 days after the leave");
    assert_eq!(case.last_present_on, day(-9), "day 0 is the last leave day");
    let unapproved = AbsenceCase::open_of(app.db(), nobody.0)
        .await
        .unwrap()
        .expect("a leave that is only requested is no leave");
    assert_eq!(unapproved.last_present_on, day(-20));
}
