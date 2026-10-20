//! Submitting a timesheet (wl-attendance §4 `submit`, §5.5 point 4, issue
//! #183): hours allocated on a day, over every timesheet and project, may not
//! exceed the attended hours; the system submit at period end runs the same
//! checks and leaves a task when it fails.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_attendance::entry::{self, NewEntry, TimesheetEntry};
use wl_attendance::submit::{self, SystemSubmit};
use wl_attendance::timesheet::HoursSource;
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus};
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, nov, project};

/// An `open` attendance timesheet of `employee` for the week of 2 November
/// for project `project_id`, with `entries` (day and minutes) against that
/// project, created by `by`.
async fn sheet(
    t: &Booted,
    employee: EmployeeId,
    project_id: i64,
    entries: &[(NaiveDate, i64)],
    by: Option<i64>,
) -> Timesheet {
    let mut new = Timesheet::regular(employee, nov(2), nov(8)).in_context(&project(project_id));
    new.created_by = by;
    let sheet = Timesheet::create(t.db(), new).await.unwrap();
    for (day, minutes) in entries {
        entry::add(
            t.state(),
            sheet.id,
            NewEntry::normal(*day, *minutes)
                .on(&project(project_id))
                .from_attendance(),
        )
        .await
        .unwrap();
    }
    sheet
}

/// `employee` attended 8 h on 2 November (an accepted record).
async fn attended(t: &Booted, employee: EmployeeId) {
    let site = t.location("Site 1").await;
    t.fixture(
        employee,
        &site,
        at(2, 7, 0),
        Some(at(2, 15, 0)),
        AttendanceStatus::Accepted,
    )
    .await;
}

async fn submit_as(t: &Booted, user: &User, sheet: &Timesheet) -> TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/submit", sheet.id), &[("x", "y")])
        .await
}

/// A attended 8 h on 2 November; T1 (Project X) has 5 h and is submitted;
/// T2 (Project Y) with 4 h is refused naming the day and the 1 h excess; with
/// 3 h it is submitted.
#[renox::test]
async fn att06_the_same_hours_are_not_allocated_twice() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    attended(&t, a).await;
    let t1 = sheet(&t, a, 10, &[(nov(2), 300)], None).await;
    let t2 = sheet(&t, a, 20, &[(nov(2), 240)], None).await;

    submit_as(&t, &keeper, &t1).await.assert_ok();
    assert_eq!(t.reload_sheet(&t1).await.status, TimesheetStatus::Submitted);
    let history = t.sheet_history(&t1).await;
    assert_eq!(history.last().unwrap().0, "submit");
    assert_eq!(history.last().unwrap().1, Some(keeper.id));
    assert_eq!(t.reload_sheet(&t1).await.submitted_by, Some(keeper.id));

    submit_as(&t, &keeper, &t2)
        .await
        .assert_status(422)
        .assert_see("2026-11-02")
        .assert_see("1 h");
    assert_eq!(t.reload_sheet(&t2).await.status, TimesheetStatus::Open);
    // Nothing was trimmed.
    let kept = TimesheetEntry::of_timesheet(t.db(), t2.id).await.unwrap();
    assert_eq!(kept[0].normal_minutes, 240);

    // Three hours instead: 5 + 3 = 8.
    TimesheetEntry::query()
        .where_(TimesheetEntry::ID.eq(kept[0].id))
        .delete(t.db())
        .await
        .unwrap();
    entry::add(
        t.state(),
        t2.id,
        NewEntry::normal(nov(2), 180).on(&project(20)).from_attendance(),
    )
    .await
    .unwrap();
    submit_as(&t, &keeper, &t2).await.assert_ok();
    assert_eq!(t.reload_sheet(&t2).await.status, TimesheetStatus::Submitted);
}

/// Hours on a day nobody attended are over the attended hours, too.
#[renox::test]
async fn att06_hours_on_a_day_without_attendance_are_refused() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    attended(&t, a).await;
    let sheet = sheet(&t, a, 10, &[(nov(2), 480), (nov(3), 120)], None).await;
    submit_as(&t, &keeper, &sheet)
        .await
        .assert_status(422)
        .assert_see("2026-11-03")
        .assert_see("2 h");
}

/// A timesheet whose hours are manual or generated (04, 10) has no
/// attendance to compare with.
#[renox::test]
async fn att06_a_manual_timesheet_is_not_checked_against_attendance() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let b = t.employee(2, &[]);
    let engagement = wl_kernel::Subject::new("accounting.engagement", 4);
    let sheet = Timesheet::create(
        t.db(),
        Timesheet::regular(b, nov(2), nov(8))
            .in_context(&engagement)
            .from_source(HoursSource::Manual),
    )
    .await
    .unwrap();
    entry::add(
        t.state(),
        sheet.id,
        NewEntry::normal(nov(2), 600).on(&engagement),
    )
    .await
    .unwrap();
    submit_as(&t, &keeper, &sheet).await.assert_ok();
    assert_eq!(t.reload_sheet(&sheet).await.status, TimesheetStatus::Submitted);
}

/// Attendance left `flagged` in the period holds the submit; an attended day
/// with no hours allocated to a cost object is refused too.
#[renox::test]
async fn att06_a_flagged_record_or_an_unallocated_day_holds_the_submit() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    t.fixture(a, &site, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Accepted)
        .await;
    let flagged = t
        .fixture(a, &site, at(3, 7, 0), Some(at(3, 15, 0)), AttendanceStatus::Flagged)
        .await;
    let sheet = sheet(&t, a, 10, &[(nov(2), 480)], None).await;
    submit_as(&t, &keeper, &sheet)
        .await
        .assert_status(422)
        .assert_see(&format!("record {}", flagged.id))
        .assert_see("2026-11-03");
    assert_eq!(t.reload_sheet(&sheet).await.status, TimesheetStatus::Open);

    // An accepted day, 4 November, with no hours allocated at all.
    let b = t.employee(2, &[]);
    t.fixture(b, &site, at(4, 7, 0), Some(at(4, 15, 0)), AttendanceStatus::Accepted)
        .await;
    let empty = sheet_for(&t, b).await;
    submit_as(&t, &keeper, &empty)
        .await
        .assert_status(422)
        .assert_see("2026-11-04")
        .assert_see("no hours allocated");
}

async fn sheet_for(t: &Booted, employee: EmployeeId) -> Timesheet {
    sheet(t, employee, 10, &[], None).await
}

/// Only a holder of `timesheets.submit` submits, and only an open timesheet.
#[renox::test]
async fn att06_submit_needs_the_permission_and_an_open_timesheet() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let nobody = t.user_with("Nadia", "reviewer").await;
    let a = t.employee(1, &[]);
    attended(&t, a).await;
    let one = sheet(&t, a, 10, &[(nov(2), 480)], None).await;
    submit_as(&t, &nobody, &one).await.assert_forbidden();
    assert_eq!(t.reload_sheet(&one).await.status, TimesheetStatus::Open);
    submit_as(&t, &keeper, &one).await.assert_ok();
    submit_as(&t, &keeper, &one).await.assert_status(422);
}

/// The system submit at period end runs the same check: when it fails the
/// timesheet stays `open` and the creator gets a task listing the days (once);
/// when it passes the timesheet is submitted with no submitter.
#[renox::test]
async fn att06_a_failed_system_submit_leaves_the_timesheet_open_and_opens_a_task() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    attended(&t, a).await;
    let t1 = sheet(&t, a, 10, &[(nov(2), 300)], None).await;
    let t2 = sheet(&t, a, 20, &[(nov(2), 240)], Some(keeper.id)).await;

    let SystemSubmit::Submitted(done) = submit::submit_by_system(t.state(), t1.id).await.unwrap()
    else {
        panic!("T1 is within the attended hours");
    };
    assert_eq!(done.status, TimesheetStatus::Submitted);
    assert_eq!(done.submitted_by, None);

    let refused = submit::submit_by_system(t.state(), t2.id).await.unwrap();
    let SystemSubmit::Refused {
        timesheet,
        message,
        task,
    } = refused
    else {
        panic!("T2 is over the attended hours");
    };
    assert_eq!(timesheet.status, TimesheetStatus::Open);
    assert!(message.contains("2026-11-02") && message.contains("1 h"), "{message}");
    assert!(task.is_some());
    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].subject, t2.subject());
    assert_eq!(opened[0].assignee, Some(wl_kernel::tickets::Assignee::user(keeper.id)));
    assert!(opened[0].description.contains("2026-11-02"));
    assert_eq!(opened[0].source_key.as_deref(), Some(submit::task_key(t2.id).as_str()));
    assert_eq!(t.reload_sheet(&t2).await.status, TimesheetStatus::Open);

    // Run again: the open task is not opened twice.
    submit::submit_by_system(t.state(), t2.id).await.unwrap();
    assert_eq!(t.tasks.opened().len(), 1);
}

/// T1 and T2 submitted in two parallel requests at the same moment: exactly
/// one is `submitted` and the other is refused naming the 1 h excess; run 20
/// times.
#[renox::test]
async fn att06_two_parallel_submits_leave_exactly_one_submitted() {
    let t = boot().await;
    for n in 0..20 {
        let a = t.employee(100 + n, &[]);
        attended(&t, a).await;
        let t1 = sheet(&t, a, 10, &[(nov(2), 300)], None).await;
        let t2 = sheet(&t, a, 20, &[(nov(2), 240)], None).await;
        let (first, second) = renox::tokio::join!(
            submit::submit_by_system(t.state(), t1.id),
            submit::submit_by_system(t.state(), t2.id),
        );
        let results = [first.unwrap(), second.unwrap()];
        let submitted = results
            .iter()
            .filter(|r| matches!(r, SystemSubmit::Submitted(_)))
            .count();
        assert_eq!(submitted, 1, "run {n}");
        for result in &results {
            if let SystemSubmit::Refused { message, .. } = result {
                assert!(
                    message.contains("2026-11-02") && message.contains("1 h"),
                    "run {n}: {message}"
                );
            }
        }
        let statuses = [
            t.reload_sheet(&t1).await.status,
            t.reload_sheet(&t2).await.status,
        ];
        assert_eq!(
            statuses
                .iter()
                .filter(|s| **s == TimesheetStatus::Submitted)
                .count(),
            1,
            "run {n}"
        );
    }
}
