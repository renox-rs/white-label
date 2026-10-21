//! BL-MPW-14 (#428), `HRM.ACL.08`: the attendance of a deployment entered in
//! error is rejected (`AttendanceReview::reject_work`). Hours already in a
//! locked timesheet need an adjustment timesheet; every other hour is
//! neither billed nor paid, so an accepted record no timesheet has taken
//! beyond `open` is rejected too.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_attendance::after_last_day::reject_work;
use wl_attendance::{
    AttendanceRecord, AttendanceStatus, Timesheet, TimesheetEntry, TimesheetStatus, WorkLocation,
};
use wl_kernel::Subject;

use support::{Booted, at, boot, nov};

/// An entry of 8 hours on `date` taken from `record`, in `sheet`.
async fn entry_of(t: &Booted, sheet: &Timesheet, record: &AttendanceRecord, date: NaiveDate) {
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: record.employee_id,
            date,
            normal_minutes: 480,
            attendance_record_id: Some(record.id),
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
}

async fn entries(t: &Booted, sheet: &Timesheet) -> usize {
    TimesheetEntry::query()
        .where_(TimesheetEntry::TIMESHEET_ID.eq(sheet.id))
        .count(t.db())
        .await
        .unwrap() as usize
}

/// AC5: five records at the deployment's location: `recorded` (day 2), `accepted`
/// in an open timesheet (day 3), `accepted` in a submitted timesheet (day 4),
/// `locked` (day 5) and one of another worker. The first two are rejected and
/// their hours leave the open timesheet; the submitted and the locked one are
/// kept for an adjustment; the other worker's is not touched.
#[renox::test]
async fn an_accepted_record_in_an_open_timesheet_is_rejected_and_a_locked_one_is_kept() {
    let t = boot().await;
    let w = t.employee(1, &[]);
    let other = t.employee(2, &[]);
    let scope = Subject::new("manpower.deployment", 5);
    let site = t
        .location_of(WorkLocation::named("D5").in_scope(&scope))
        .await;
    let mark = |day: u32, status: AttendanceStatus| {
        t.fixture(w, &site, at(day, 7, 0), Some(at(day, 15, 0)), status)
    };
    let recorded = mark(2, AttendanceStatus::Recorded).await;
    let accepted_open = mark(3, AttendanceStatus::Accepted).await;
    let accepted_submitted = mark(4, AttendanceStatus::Accepted).await;
    let locked = mark(5, AttendanceStatus::Locked).await;
    let strangers = t
        .fixture(
            other,
            &site,
            at(3, 7, 0),
            Some(at(3, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    let open = t
        .sheet_in(Timesheet::regular(w, nov(2), nov(3)), TimesheetStatus::Open)
        .await;
    let submitted = t
        .sheet_in(
            Timesheet::regular(w, nov(4), nov(4)),
            TimesheetStatus::Submitted,
        )
        .await;
    entry_of(&t, &open, &accepted_open, nov(3)).await;
    entry_of(&t, &submitted, &accepted_submitted, nov(4)).await;

    let done = reject_work(t.state(), &[scope], w, nov(1), "entered in error")
        .await
        .unwrap();

    assert_eq!(
        done.rejected,
        [recorded.record_id(), accepted_open.record_id()]
    );
    assert_eq!(
        done.kept,
        [accepted_submitted.record_id(), locked.record_id()]
    );
    assert_eq!(t.reload(&recorded).await.status, AttendanceStatus::Rejected);
    assert_eq!(
        t.reload(&accepted_open).await.status,
        AttendanceStatus::Rejected
    );
    assert_eq!(
        t.reload(&accepted_submitted).await.status,
        AttendanceStatus::Accepted
    );
    assert_eq!(t.reload(&locked).await.status, AttendanceStatus::Locked);
    assert_eq!(
        t.reload(&strangers).await.status,
        AttendanceStatus::Accepted
    );
    assert_eq!(
        entries(&t, &open).await,
        0,
        "the hours leave the open sheet"
    );
    assert_eq!(entries(&t, &submitted).await, 1);

    // Asked again, nothing more is rejected and the same two are kept.
    let again = reject_work(
        t.state(),
        &[Subject::new("manpower.deployment", 5)],
        w,
        nov(1),
        "entered in error",
    )
    .await
    .unwrap();
    assert!(again.rejected.is_empty());
    assert_eq!(again.kept.len(), 2);
}
