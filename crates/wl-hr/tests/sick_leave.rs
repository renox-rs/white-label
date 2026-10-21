//! BL-HRM-02 (#192): `record_sick`. An employee who calls in sick today is
//! recorded as approved sick leave by the employee's manager (without
//! `leave.approve`) or by HR: `LeaveApproved` goes out at once, a start date
//! other than today is refused, and the sick note may follow. With no note by
//! the end of the 2nd working day after the start, HR gets one task at 00:00
//! the next day; attaching the note closes it, or means there is none.

mod support;

use renox::prelude::*;
use wl_hr::sick_leave::{self, TASK};
use wl_hr::{Employee, EmployeeStatus, LeaveRequest, LeaveStatus};
use wl_kernel::tickets::Assignee;

use support::{Booted, branch_scope_of, nov, other_branch};

/// `user` posts the sick leave of `employee` on `day`.
async fn record(
    t: &Booted,
    day: u32,
    user: &User,
    employee: &Employee,
    from: &str,
    to: &str,
) -> renox::testing::TestResponse {
    t.on(nov(day), async {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(
                &format!("/hr/employees/{}/sick-leave", employee.id),
                &[("from_date", from), ("to_date", to)],
            )
            .await
    })
    .await
}

/// `user` attaches note 77 to request `request` on `day`.
async fn attach(t: &Booted, day: u32, user: &User, request: &LeaveRequest) -> renox::testing::TestResponse {
    t.on(nov(day), async {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(
                &format!("/hr/leave/{}/sick-note", request.id),
                &[("document", "77")],
            )
            .await
    })
    .await
}

/// The sick leave of `employee`.
async fn leave_of(t: &Booted, employee: &Employee) -> LeaveRequest {
    LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("a leave request")
}

/// Runs the daily job as the command does for `date`.
async fn daily(t: &Booted, date: &str) {
    t.app
        .kernel()
        .call(TASK, vec!["--date".to_owned(), date.to_owned()])
        .await
        .expect("the daily leave job runs");
}

/// The manager of branch 1 records the sick leave of a driver of branch 1;
/// the manager of branch 2 is refused.
#[renox::test]
async fn the_manager_of_the_branch_records_it_and_another_manager_is_refused() {
    let t = support::boot().await;
    let (employee, _) = t.ravi().await;
    let manager = t.user_in_branch("Mona", "manager").await;
    let elsewhere = t.user("Mark").await;
    elsewhere
        .assign_role_in(t.db(), "manager", &branch_scope_of(&other_branch()))
        .await
        .unwrap();

    record(&t, 2, &elsewhere, &employee, "2026-11-02", "2026-11-03")
        .await
        .assert_forbidden();
    assert!(t.leave_approved.all().is_empty());
    assert!(LeaveRequest::query().get(t.db()).await.unwrap().is_empty());

    record(&t, 2, &manager, &employee, "2026-11-02", "2026-11-03")
        .await
        .assert_ok();
    let request = leave_of(&t, &employee).await;
    assert_eq!(request.status, LeaveStatus::Approved);
    assert!(request.recorded_sick);
    assert_eq!(request.leave_type, "sick");
    assert_eq!((request.from_date, request.to_date), (nov(2), nov(3)));
    assert_eq!(request.decided_by, Some(manager.id));
    // Hired on 2 Nov: in probation, so both days are unpaid.
    assert_eq!(request.unpaid_days, 2);
    // The note may follow: due by the 2nd working day after the start.
    assert_eq!(request.sick_note_due_on, Some(nov(4)));
    let events = t.leave_approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, employee.employee_id());
    assert_eq!(events[0].request, request.id);
    assert_eq!(events[0].leave_type, "sick");
    assert_eq!((events[0].from, events[0].to), (nov(2), nov(3)));
}

/// HR (`leave.approve`) records it too; a plain user is refused.
#[renox::test]
async fn hr_with_leave_approve_records_it_and_a_user_without_a_role_is_refused() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    record(&t, 2, &account, &employee, "2026-11-02", "2026-11-02")
        .await
        .assert_forbidden();
    record(&t, 2, &hr, &employee, "2026-11-02", "2026-11-02")
        .await
        .assert_ok();
    assert_eq!(leave_of(&t, &employee).await.status, LeaveStatus::Approved);
}

/// A start date other than today is refused.
#[renox::test]
async fn a_start_date_other_than_today_is_refused() {
    let t = support::boot().await;
    let (employee, _) = t.ravi().await;
    let manager = t.user_in_branch("Mona", "manager").await;
    record(&t, 2, &manager, &employee, "2026-11-03", "2026-11-03")
        .await
        .assert_status(422);
    record(&t, 2, &manager, &employee, "2026-11-01", "2026-11-03")
        .await
        .assert_status(422);
    record(&t, 2, &manager, &employee, "2026-11-02", "2026-11-01")
        .await
        .assert_status(422);
    assert!(LeaveRequest::query().get(t.db()).await.unwrap().is_empty());
    assert!(t.leave_approved.all().is_empty());
}

/// The service takes the same decisions without the route.
#[renox::test]
async fn the_service_records_for_today_only() {
    let t = support::boot().await;
    let (employee, _) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let refused = t
        .on(
            nov(2),
            sick_leave::record(t.state(), hr.id, employee.id, nov(3), nov(3)),
        )
        .await;
    assert!(refused.is_err());
    let recorded = t
        .on(
            nov(2),
            sick_leave::record(t.state(), hr.id, employee.id, nov(2), nov(2)),
        )
        .await
        .unwrap();
    assert_eq!(recorded.status, LeaveStatus::Approved);
}

/// No note by 23:59 on Wednesday 4 Nov: one task for HR at 00:00 on 5 Nov;
/// a note attached on the 4th means no task; attaching it later closes it.
#[renox::test]
async fn hr_gets_one_task_when_the_note_does_not_follow_and_attaching_it_closes_it() {
    let t = support::boot().await;
    let (without, _) = t.ravi().await;
    let with_note = t.employee_in(51, EmployeeStatus::Active).await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let manager = t.user_in_branch("Mona", "manager").await;

    record(&t, 2, &manager, &without, "2026-11-02", "2026-11-03")
        .await
        .assert_ok();
    record(&t, 2, &hr, &with_note, "2026-11-02", "2026-11-03")
        .await
        .assert_ok();
    let gets_note = leave_of(&t, &with_note).await;
    attach(&t, 4, &hr, &gets_note).await.assert_ok();
    assert_eq!(
        leave_of(&t, &with_note).await.sick_note_document_id,
        Some(77)
    );

    // The note is still due on the 4th: no task at 00:00 that day.
    daily(&t, "2026-11-04").await;
    assert!(t.tasks.opened().is_empty());

    // 00:00 on the 5th: one task, for the leave with no note.
    daily(&t, "2026-11-05").await;
    let missing = leave_of(&t, &without).await;
    let tasks = t.tasks.with_key("hr.sick_note:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(sick_leave::source_key(missing.id).as_str())
    );
    assert_eq!(tasks[0].assignee, Some(Assignee::user(hr.id)));
    assert_eq!(tasks[0].subject, missing.subject());
    let missing = leave_of(&t, &without).await;
    assert_eq!(missing.sick_note_task_id, Some(1));
    assert_eq!(missing.sick_note_task_raised_on, Some(nov(5)));
    assert!(!t.tasks.is_resolved(1));

    // A second run the same day, or the next, raises nothing more.
    daily(&t, "2026-11-05").await;
    daily(&t, "2026-11-06").await;
    assert_eq!(t.tasks.opened().len(), 1);

    // Attaching the note closes the task.
    attach(&t, 6, &hr, &missing).await.assert_ok();
    assert!(t.tasks.is_resolved(1));
    let done = leave_of(&t, &without).await;
    assert_eq!(done.sick_note_document_id, Some(77));
    assert_eq!(done.sick_note_attached_on, Some(nov(6)));
    daily(&t, "2026-11-09").await;
    assert_eq!(t.tasks.opened().len(), 1);
}

/// The note is due on working days: a leave that starts on a Thursday is due
/// the next Monday, so there is no task on the Saturday.
#[renox::test]
async fn the_working_calendar_decides_the_day_the_task_is_raised() {
    let t = support::boot().await;
    let (employee, _) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    record(&t, 5, &hr, &employee, "2026-11-05", "2026-11-05")
        .await
        .assert_ok();
    let request = leave_of(&t, &employee).await;
    assert_eq!(request.sick_note_due_on, Some(nov(9)));
    for day in ["2026-11-07", "2026-11-09"] {
        daily(&t, day).await;
        assert!(t.tasks.opened().is_empty(), "{day}");
    }
    daily(&t, "2026-11-10").await;
    assert_eq!(t.tasks.opened().len(), 1);
}

/// A longer wait: the setting changes the number of working days.
#[renox::test]
async fn the_number_of_days_is_a_setting() {
    let t = support::boot_with(wl_hr::WlHr::new().sick_note_days(1), true).await;
    let (employee, _) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    record(&t, 2, &hr, &employee, "2026-11-02", "2026-11-02")
        .await
        .assert_ok();
    assert_eq!(leave_of(&t, &employee).await.sick_note_due_on, Some(nov(3)));
    daily(&t, "2026-11-04").await;
    assert_eq!(t.tasks.opened().len(), 1);
}
