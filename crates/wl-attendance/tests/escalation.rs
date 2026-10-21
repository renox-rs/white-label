//! BL-ATT-10 (#187): `HRM.RUL.15` at E days, the escalation of an external
//! timesheet nobody answers (wl-attendance §5.6, §6 *Automations*, §7).

mod support;

use renox::db::DateTime;
use renox::prelude::*;
use wl_attendance::escalation;
use wl_attendance::timesheet::ApproverKind;
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, nov, post_to, users_of};

const ESCALATED: &str = "attendance.timesheet_escalated";

fn deployment() -> Subject {
    Subject::new("manpower.deployment", 9)
}

/// An external timesheet for the deployment, `submitted` at `submitted` and
/// still waiting for the client approver.
async fn waiting(t: &Booted, employee: EmployeeId, submitted: DateTime) -> Timesheet {
    let mut new = Timesheet::regular(employee, nov(1), nov(1)).in_context(&deployment());
    new.approver_kind = ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    new.submitted_at = Some(submitted);
    t.sheet_in(new, TimesheetStatus::Submitted).await
}

/// The people of a manpower contract: an account manager (an employee whose
/// line manager is `Mona`), the client's escalation contact `Cleo` and the
/// client approver `Carl`.
struct Cast {
    account_manager: User,
    manager: User,
    contact: User,
    client: User,
}

async fn cast(t: &Booted) -> Cast {
    let manager = t.user_with("Mona", "hr").await;
    let account_manager = t.user_with("Amal", "account_manager").await;
    let contact = t.user_with("Cleo", "client_approver").await;
    let client = t.user_with("Carl", "client_approver").await;
    let am = t.employee(50, &[manager.id]);
    t.staff.account(account_manager.id, am);
    t.approvers.account_manager(account_manager.id);
    t.approvers.escalation_contact(contact.id);
    Cast {
        account_manager,
        manager,
        contact,
        client,
    }
}

async fn run_on(t: &Booted, day: u32) -> escalation::Report {
    t.on(nov(day), async {
        escalation::run(t.state(), nov(day)).await
    })
    .await
    .unwrap()
}

/// AC1: given an external timesheet submitted 2026-11-01 09:00 and still
/// `submitted`, at the 08:00 run on 2026-11-07 the escalation contact and the
/// account manager's manager are notified and one task is created for the
/// account manager; the run on 2026-11-06 sends nothing. AC3: running the rule
/// again on 2026-11-08 sends nothing new. The state does not change.
#[renox::test]
async fn att10_at_e_days_the_contact_and_the_managers_manager_are_told_once() {
    let t = boot().await;
    let who = cast(&t).await;
    let sheet = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    // The scheduled task itself, the day before: nothing.
    t.on(nov(6), async {
        t.app.run_scheduled(escalation::TASK).await
    })
    .await
    .unwrap();
    assert!(t.notices.of_type(ESCALATED).is_empty());
    assert!(t.tasks.opened().is_empty());

    t.on(nov(7), async {
        t.app.run_scheduled(escalation::TASK).await
    })
    .await
    .unwrap();
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    let mut recipients = users_of(&told[0]);
    recipients.sort_unstable();
    let mut wanted = vec![who.contact.id, who.manager.id];
    wanted.sort_unstable();
    assert_eq!(recipients, wanted);
    assert_eq!(told[0].subject, sheet.subject());
    assert_eq!(told[0].data["days"], 6);
    assert_eq!(told[0].data["submitted"], "2026-11-01");

    let tasks = t.tasks.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, sheet.subject());
    assert_eq!(
        tasks[0].assignee,
        Some(wl_kernel::tickets::Assignee::user(who.account_manager.id))
    );
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(escalation::task_key(sheet.id).as_str())
    );
    assert!(tasks[0].description.contains("signed sheet"));
    assert!(!tasks[0].description.contains("No line manager"));

    // Again, the same day and the day after: nothing new.
    for day in [7, 8] {
        let report = run_on(&t, day).await;
        assert_eq!(report.escalated, 0);
    }
    assert_eq!(t.notices.of_type(ESCALATED).len(), 1);
    assert_eq!(t.tasks.opened().len(), 1);

    // The escalation changes nothing on the timesheet.
    let stored = t.reload_sheet(&sheet).await;
    assert_eq!(stored.status, TimesheetStatus::Submitted);
    assert!(stored.escalated_at.is_some());
    assert!(t.approved.all().is_empty());
}

/// AC2: given the account manager has no line manager, the users with
/// `timesheets.approve_on_behalf` in scope are notified and the task says so.
#[renox::test]
async fn att10_without_a_line_manager_the_users_who_may_approve_on_behalf_are_told() {
    let t = boot().await;
    let who = cast(&t).await;
    // The account manager's employee has no manager.
    t.staff
        .change(EmployeeId(50), |profile| profile.managers.clear());
    let other = t.user_with("Omar", "account_manager").await;
    waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    let report = run_on(&t, 7).await;
    assert_eq!(report.escalated, 1);
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    let recipients = users_of(&told[0]);
    assert!(recipients.contains(&other.id));
    assert!(recipients.contains(&who.account_manager.id));
    assert!(recipients.contains(&who.contact.id));
    assert!(!recipients.contains(&who.manager.id));
    let tasks = t.tasks.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains("No line manager"));
}

/// Without an escalation contact on the contract only the manager is told and
/// the task says so.
#[renox::test]
async fn att10_without_an_escalation_contact_only_the_manager_is_told() {
    let t = boot().await;
    let manager = t.user_with("Mona", "hr").await;
    let account_manager = t.user_with("Amal", "account_manager").await;
    let am = t.employee(50, &[manager.id]);
    t.staff.account(account_manager.id, am);
    t.approvers.account_manager(account_manager.id);
    waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    run_on(&t, 7).await;
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![manager.id]);
    let tasks = t.tasks.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains("No escalation contact"));
}

/// AC4: if the timesheet is approved on 2026-11-05, nothing is sent and no
/// task exists; and a task opened at E closes by itself on approval.
#[renox::test]
async fn att10_an_approved_timesheet_is_not_escalated_and_its_task_closes() {
    let t = boot().await;
    let who = cast(&t).await;
    let early = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;
    let late = waiting(&t, t.employee(2, &[]), at(1, 9, 0)).await;

    // The client approves `early` on the 5th.
    t.on(nov(5), async {
        post_to(&t, &who.client, &early, "approve", &[("x", "y")])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(
        t.reload_sheet(&early).await.status,
        TimesheetStatus::Approved
    );

    let report = run_on(&t, 7).await;
    assert_eq!(report.escalated, 1, "only `late` is escalated");
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject, late.subject());
    let tasks = t.tasks.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, late.subject());
    assert_eq!(t.tasks.closed_with_key(&escalation::task_key(late.id)), 0);

    // `late` is approved: its task closes by itself.
    t.on(nov(8), async {
        post_to(&t, &who.client, &late, "approve", &[("x", "y")])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(t.tasks.closed_with_key(&escalation::task_key(late.id)), 1);
}

/// AC5: after `reject` and a new `submit` on 2026-11-06 the escalation comes
/// on 2026-11-12, six days after the new `submit`.
#[renox::test]
async fn att10_after_a_reject_and_a_new_submit_the_clock_restarts() {
    let t = boot().await;
    let who = cast(&t).await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let sheet = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    assert_eq!(run_on(&t, 5).await.escalated, 0);
    t.on(nov(6), async {
        post_to(
            &t,
            &who.client,
            &sheet,
            "reject",
            &[("reason", "wrong site")],
        )
        .await
        .assert_ok();
        assert_eq!(t.reload_sheet(&sheet).await.status, TimesheetStatus::Open);
        post_to(&t, &keeper, &sheet, "submit", &[("x", "y")])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(
        t.reload_sheet(&sheet).await.status,
        TimesheetStatus::Submitted
    );

    for day in [7, 11] {
        assert_eq!(run_on(&t, day).await.escalated, 0, "day {day}");
    }
    assert!(t.notices.of_type(ESCALATED).is_empty());
    assert_eq!(run_on(&t, 12).await.escalated, 1);
    assert_eq!(t.notices.of_type(ESCALATED).len(), 1);
    assert_eq!(t.tasks.opened().len(), 1);
}

/// A timesheet an internal approver approves is not escalated by this rule,
/// and a changed E applies to the timesheets that wait.
#[renox::test]
async fn att10_internal_timesheets_are_left_alone_and_e_is_a_setting() {
    let t = support::boot_with(wl_attendance::WlAttendance::new().escalation_days(3), "").await;
    cast(&t).await;
    let worker = t.employee(1, &[]);
    let mut internal = Timesheet::regular(worker, nov(1), nov(1)).in_context(&deployment());
    internal.submitted_at = Some(at(1, 9, 0));
    t.sheet_in(internal, TimesheetStatus::Submitted).await;
    let external = waiting(&t, t.employee(2, &[]), at(1, 9, 0)).await;

    assert_eq!(run_on(&t, 3).await.escalated, 0, "E is 3: day 4 is the day");
    assert_eq!(run_on(&t, 4).await.escalated, 1);
    let told = t.notices.of_type(ESCALATED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject, external.subject());
}
