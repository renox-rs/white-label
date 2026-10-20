//! What an accepted record gives the open timesheet, and the entries
//! themselves (wl-attendance §3 *Timesheet entry*, §4 `accept` effects,
//! issue #182): `accept` proposes the worked minutes, non-billable for a
//! `permit_invalid` record; the entries entered by hand are checked and feed
//! the overtime cap; `lock` and `reopen` of the timesheet.

mod support;

use renox::prelude::*;
use wl_attendance::entry::{self, Category, NewEntry, TimesheetEntry, TimesheetEntryHours};
use wl_attendance::overtime::EntryHours;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::timesheet::{self, Consumer, HoursSource};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus, WorkLocation};
use wl_kernel::workforce::EmployeeId;

use support::{at, boot, nov, project};

/// A form with no fields.
const NONE: &[(&str, &str)] = &[];

/// `user` posts `form` to the reopen route of `sheet`.
async fn reopen(
    t: &support::Booted,
    sheet: &Timesheet,
    form: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/reopen", sheet.id), form)
        .await
}

/// An `open` attendance timesheet of `employee` for the week of 2 November.
async fn week(t: &support::Booted, employee: EmployeeId) -> Timesheet {
    Timesheet::create(t.db(), Timesheet::regular(employee, nov(2), nov(8)))
        .await
        .unwrap()
}

async fn entries_of(t: &support::Booted, sheet: &Timesheet) -> Vec<TimesheetEntry> {
    TimesheetEntry::of_timesheet(t.db(), sheet.id).await.unwrap()
}

/// `accept` proposes the eight hours on the open timesheet, against the
/// project of the location, billable, from attendance, linked to the record.
#[renox::test]
async fn att05_an_accepted_record_proposes_its_hours_on_the_open_timesheet() {
    let t = boot().await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").in_scope(&project(5)))
        .await;
    let sheet = week(&t, a).await;
    let record = t
        .fixture(a, &site, at(3, 7, 0), Some(at(3, 16, 0)), AttendanceStatus::Recorded)
        .await;
    // One hour break: 8 h worked.
    set_break(&t, &record).await;
    t.on(nov(3), async {
        t.app.acting_as(&c);
        t.app
            .htmx()
            .post(&format!("/attendance/records/{}/accept", record.id), NONE)
            .await
            .assert_ok();
    })
    .await;
    let entries = entries_of(&t, &sheet).await;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.normal_minutes, 480);
    assert_eq!(entry.total_minutes(), 480);
    assert!(entry.billable);
    assert_eq!(entry.source, HoursSource::Attendance);
    assert_eq!(entry.attendance_record_id, Some(record.id));
    assert_eq!(entry.cost_object(), Some(project(5)));
    assert_eq!(entry.date, nov(3));
    assert_eq!(entry.employee_id, a.0);
}

/// Sets a one-hour break on a fixture record (the fixture has none).
async fn set_break(t: &support::Booted, record: &wl_attendance::AttendanceRecord) {
    wl_attendance::AttendanceRecord::query()
        .where_(wl_attendance::AttendanceRecord::ID.eq(record.id))
        .update(
            t.db(),
            &[(
                "breaks",
                &renox::db::Json(vec![wl_attendance::BreakSpan::hm((12, 0), (13, 0))]),
            )],
        )
        .await
        .unwrap();
}

/// The hours of a `permit_invalid` record are proposed non-billable once HR
/// accepts it; none are proposed while it is flagged.
#[renox::test]
async fn att05_a_permit_invalid_record_proposes_non_billable_hours_after_the_review() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let hr = t.user_with("Hana", "reviewer").await;
    let a = t.employee(1, &[]);
    t.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(1)));
    let site = t.location("Site 1").await;
    let sheet = week(&t, a).await;
    t.on(nov(2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert!(entries_of(&t, &sheet).await.is_empty());
        t.app.acting_as(&hr);
        t.app
            .htmx()
            .post(
                &format!("/attendance/records/{}/accept", record.id),
                &[("reason", "renewal verified")],
            )
            .await
            .assert_ok();
    })
    .await;
    let entries = entries_of(&t, &sheet).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].normal_minutes, 480);
    assert!(!entries[0].billable);
}

/// With no open timesheet nothing is proposed; a submitted one is not
/// touched.
#[renox::test]
async fn att05_nothing_is_proposed_without_an_open_timesheet() {
    let t = boot().await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    let submitted = t
        .sheet_in(
            Timesheet::regular(a, nov(2), nov(8)),
            TimesheetStatus::Submitted,
        )
        .await;
    let record = t
        .fixture(a, &site, at(3, 7, 0), Some(at(3, 15, 0)), AttendanceStatus::Recorded)
        .await;
    t.on(nov(3), async {
        t.app.acting_as(&c);
        t.app
            .htmx()
            .post(&format!("/attendance/records/{}/accept", record.id), NONE)
            .await
            .assert_ok();
    })
    .await;
    assert!(entries_of(&t, &submitted).await.is_empty());
    assert_eq!(t.reload(&record).await.status, AttendanceStatus::Accepted);
}

/// A correction moves the proposed hours; a check-out on an accepted record
/// with no hours yet proposes them; `unpropose_in` removes them from an open
/// timesheet only.
#[renox::test]
async fn att05_the_proposed_hours_follow_the_record() {
    let t = boot().await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    let sheet = week(&t, a).await;
    let record = t
        .fixture(a, &site, at(3, 7, 0), Some(at(3, 15, 0)), AttendanceStatus::Recorded)
        .await;
    t.on(nov(3), async {
        t.app.acting_as(&c);
        t.app
            .htmx()
            .post(&format!("/attendance/records/{}/accept", record.id), NONE)
            .await
            .assert_ok();
        assert_eq!(entries_of(&t, &sheet).await[0].normal_minutes, 480);

        t.app
            .htmx()
            .post(
                &format!("/attendance/records/{}/correct", record.id),
                &[
                    ("check_in", "2026-11-03T07:00"),
                    ("check_out", "2026-11-03T13:00"),
                    ("reason", "left early"),
                ],
            )
            .await
            .assert_ok();
        let entries = entries_of(&t, &sheet).await;
        assert_eq!(entries.len(), 1, "the entry is updated, not added");
        assert_eq!(entries[0].normal_minutes, 360);

        // An accepted record still open proposes nothing until it is closed.
        let open = t
            .fixture(a, &site, at(4, 7, 0), None, AttendanceStatus::Accepted)
            .await;
        assert_eq!(entries_of(&t, &sheet).await.len(), 1);
        service::check_out(t.state(), open.id, at(4, 15, 0))
            .await
            .unwrap();
        let entries = entries_of(&t, &sheet).await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].normal_minutes, 480);
        assert_eq!(entries[1].attendance_record_id, Some(open.id));

        // Taken off the open timesheet.
        let mut tx = t.db().begin().await.unwrap();
        entry::unpropose_in(&mut tx, open.id).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(entries_of(&t, &sheet).await.len(), 1);
    })
    .await;
}

/// Entries by hand: only on an open timesheet, with hours, not negative, in
/// the period.
#[renox::test]
async fn att05_an_entry_is_checked_on_save() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let open = week(&t, a).await;
    let closed = t
        .sheet_in(
            Timesheet::regular(a, nov(2), nov(8)),
            TimesheetStatus::Submitted,
        )
        .await;
    let day = nov(3);
    assert!(entry::add(t.state(), open.id, NewEntry::normal(day, 0)).await.is_err());
    assert!(entry::add(t.state(), open.id, NewEntry::normal(day, -60)).await.is_err());
    assert!(entry::add(t.state(), open.id, NewEntry::normal(nov(9), 60)).await.is_err());
    assert!(entry::add(t.state(), closed.id, NewEntry::normal(day, 60)).await.is_err());
    let saved = entry::add(
        t.state(),
        open.id,
        NewEntry::of(day, Category::Overtime, 90).on(&project(5)).note(" late pour "),
    )
    .await
    .unwrap();
    assert_eq!(saved.overtime_minutes, 90);
    assert_eq!(saved.normal_minutes, 0);
    assert_eq!(saved.note, "late pour");
    assert_eq!(saved.cost_object(), Some(project(5)));
    assert_eq!(saved.employee_id, a.0);
    assert_eq!(saved.source, HoursSource::Manual);
}

/// `TimesheetEntryHours` tells the overtime cap the manual and generated
/// minutes of regular timesheets, not the ones proposed from attendance.
#[renox::test]
async fn att05_only_manual_and_generated_entries_count_for_the_cap() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let sheet = Timesheet::create(
        t.db(),
        Timesheet::regular(a, nov(2), nov(8)).from_source(HoursSource::Manual),
    )
    .await
    .unwrap();
    entry::add(t.state(), sheet.id, NewEntry::normal(nov(3), 120)).await.unwrap();
    let mut generated = NewEntry::normal(nov(3), 60);
    generated.source = HoursSource::Generated;
    entry::add(t.state(), sheet.id, generated).await.unwrap();
    entry::add(t.state(), sheet.id, NewEntry::normal(nov(3), 480).from_attendance())
        .await
        .unwrap();
    let minutes = TimesheetEntryHours
        .minutes_on(t.state(), a, nov(3))
        .await
        .unwrap();
    assert_eq!(minutes, 180);
    assert_eq!(
        TimesheetEntryHours.minutes_on(t.state(), a, nov(4)).await.unwrap(),
        0
    );
}

/// `reopen` needs `timesheets.reopen` and a reason; an approved timesheet
/// that nothing used goes back to `open`; once locked or consumed it never
/// reopens (`HRM.ACL.08`).
#[renox::test]
async fn att05_reopen_is_refused_on_a_locked_or_consumed_timesheet() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let nobody = t.user_with("Nadia", "reviewer").await;
    let a = t.employee(1, &[]);
    let approved = t
        .sheet_in(Timesheet::regular(a, nov(2), nov(8)), TimesheetStatus::Approved)
        .await;
    t.app.acting_as(&nobody);
    reopen(&t, &approved, &[("reason", "typo")])
        .await
        .assert_forbidden();
    t.app.acting_as(&keeper);
    reopen(&t, &approved, NONE).await.assert_status(422);
    assert_eq!(t.reload_sheet(&approved).await.status, TimesheetStatus::Approved);
    reopen(&t, &approved, &[("reason", "forgot a day")])
        .await
        .assert_ok();
    assert_eq!(t.reload_sheet(&approved).await.status, TimesheetStatus::Open);
    let history = t.sheet_history(&approved).await;
    let reopened = history.iter().find(|h| h.0 == "reopen").unwrap();
    assert_eq!(reopened.1, Some(keeper.id));
    assert_eq!(reopened.2.as_deref(), Some("forgot a day"));

    // Locked by its first consumer: terminal.
    let used = t
        .sheet_in(Timesheet::regular(a, nov(9), nov(15)), TimesheetStatus::Approved)
        .await;
    let locked = timesheet::lock(t.state(), used.id, Consumer::Payroll, "hr.payroll_run:5")
        .await
        .unwrap();
    assert_eq!(locked.status, TimesheetStatus::Locked);
    assert_eq!(locked.paid_by.as_deref(), Some("hr.payroll_run:5"));
    assert!(locked.consumed());
    reopen(&t, &locked, &[("reason", "again")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_sheet(&locked).await.status, TimesheetStatus::Locked);

    // Approved but provisionally paid: consumed, so not reopened.
    let mut provisional = Timesheet::regular(a, nov(16), nov(22));
    provisional.paid_provisionally_by = Some("hr.payroll_run:6".into());
    let provisional = t.sheet_in(provisional, TimesheetStatus::Approved).await;
    reopen(&t, &provisional, &[("reason", "again")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload_sheet(&provisional).await.status,
        TimesheetStatus::Approved
    );

    // Only an approved timesheet locks, and it needs a consumer.
    let open = week(&t, a).await;
    assert!(
        timesheet::lock(t.state(), open.id, Consumer::Export, "export")
            .await
            .is_err()
    );
    let exported = t
        .sheet_in(Timesheet::regular(a, nov(23), nov(29)), TimesheetStatus::Approved)
        .await;
    let exported = timesheet::lock(t.state(), exported.id, Consumer::Export, "export:1")
        .await
        .unwrap();
    assert!(exported.exported_at.is_some());
}
