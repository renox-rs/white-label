//! Closing the open timesheet of a context at its last day (09 Manpower
//! `end`, BL-MPW-06 (#420)): the kernel's `TimesheetClosing` that
//! wl-attendance provides.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_attendance::entry::{self, NewEntry};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus};
use wl_kernel::timesheets::timesheet_closing;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, nov, project};

/// An `open` timesheet of `employee` from `from` to `to` for project 10 with
/// 8 h on its first day against the project.
async fn sheet(t: &Booted, employee: EmployeeId, from: NaiveDate, to: NaiveDate) -> Timesheet {
    let sheet = Timesheet::create(
        t.db(),
        Timesheet::regular(employee, from, to).in_context(&project(10)),
    )
    .await
    .unwrap();
    entry::add(
        t.state(),
        sheet.id,
        NewEntry::normal(from, 480)
            .on(&project(10))
            .from_attendance(),
    )
    .await
    .unwrap();
    sheet
}

/// `employee` attended 8 h on 2 November.
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

/// AC2: a week that runs past the last day is cut to it and submitted at
/// once; asked again nothing changes.
#[renox::test]
async fn an_open_timesheet_is_cut_to_the_last_day_and_submitted_once() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    attended(&t, a).await;
    let open = sheet(&t, a, nov(2), nov(8)).await;

    let closing = timesheet_closing(t.state());
    let closed = closing
        .close_open(t.state(), &project(10), a, nov(5))
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert!(closed[0].submitted);
    assert_eq!(closed[0].period_to, nov(5));
    let after = t.reload_sheet(&open).await;
    assert_eq!(after.status, TimesheetStatus::Submitted);
    assert_eq!(after.period_to, nov(5));

    let again = closing
        .close_open(t.state(), &project(10), a, nov(5))
        .await
        .unwrap();
    assert!(again.is_empty(), "a submitted timesheet is not touched");
}

/// AC2: a timesheet a check refuses stays open and says why; a timesheet of
/// another owner or one that starts after the last day is left alone.
#[renox::test]
async fn a_refused_timesheet_stays_open_and_others_are_left_alone() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    // No attendance: the 8 h on 2 November exceed the attended hours.
    let refused = sheet(&t, a, nov(2), nov(8)).await;
    let later = sheet(&t, a, nov(9), nov(15)).await;
    let other = sheet(&t, b, nov(2), nov(8)).await;

    let closed = timesheet_closing(t.state())
        .close_open(t.state(), &project(10), a, nov(5))
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
    assert!(!closed[0].submitted);
    assert!(
        closed[0]
            .message
            .as_deref()
            .is_some_and(|m| m.contains("2026-11-02"))
    );
    assert_eq!(t.reload_sheet(&refused).await.status, TimesheetStatus::Open);
    assert_eq!(t.reload_sheet(&later).await.period_from, nov(9));
    assert_eq!(t.reload_sheet(&later).await.status, TimesheetStatus::Open);
    assert_eq!(t.reload_sheet(&other).await.status, TimesheetStatus::Open);
}
