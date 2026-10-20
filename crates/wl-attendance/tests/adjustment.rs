//! The adjustment timesheet (wl-attendance §4 `open_adjustment`, §5.8,
//! `HRM.ACL.08`, issue #184): a locked timesheet is never edited or
//! reopened; a correction is a new timesheet of kind `adjustment` with the
//! signed difference, a reason, an origin and where it flows.

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_attendance::entry::{self, Category, NewEntry, TimesheetEntry};
use wl_attendance::timesheet::{AdjustmentOrigin, Affects, TimesheetKind};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, nov, project};

fn cost_code() -> Subject {
    Subject::new("construction.cost_code", 3)
}

/// The cost object of the examples, as a form sends it.
const COST_CODE: &str = "construction.cost_code:3";

/// A timesheet of `employee` in `status` with 8 h of normal hours on 2
/// November against the cost code, used by payroll run R.
async fn eight_hours(t: &Booted, employee: EmployeeId, status: TimesheetStatus) -> Timesheet {
    let mut new = Timesheet::regular(employee, nov(2), nov(8)).in_context(&project(5));
    new.paid_by = Some("hr.payroll_run:7".into());
    let sheet = t.sheet_in(new, status).await;
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: employee.0,
            date: nov(2),
            cost_object_type: Some(cost_code().subject_type),
            cost_object_id: Some(cost_code().subject_id),
            normal_minutes: 480,
            billable: true,
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
    sheet
}

async fn adjust(t: &Booted, user: &User, sheet: &Timesheet, form: &[(&str, &str)]) -> TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/adjustments", sheet.id), form)
        .await
}

/// The entries JSON of one signed entry on 2 November.
fn entries(category: &str, minutes: i64) -> String {
    format!(
        r#"[{{"date": "2026-11-02", "category": "{category}", "minutes": {minutes}, "cost_object": "{COST_CODE}"}}]"#
    )
}

/// A payroll-error adjustment with the entries `json`.
async fn adjust_json(t: &Booted, user: &User, sheet: &Timesheet, json: String) -> TestResponse {
    adjust(
        t,
        user,
        sheet,
        &[
            ("reason", "too many hours"),
            ("origin", "payroll_error"),
            ("affects", "payroll"),
            ("entries", json.as_str()),
        ],
    )
    .await
}

async fn adjustments_of(t: &Booted, sheet: &Timesheet) -> Vec<Timesheet> {
    sheet.adjusted_by(t.db()).await.unwrap()
}

/// T is `locked` by payroll run R with 8 h normal on 2 November: user A with
/// `timesheets.adjust` opens an adjustment (origin `attendance_error`,
/// affects `both`, a reason, one entry of -8 h): an `open` timesheet of kind
/// `adjustment` is linked to T and T's `adjusted_by` lists it; T's entries,
/// status and consumers are unchanged, and `reopen` on T is refused.
#[renox::test]
async fn att07_a_locked_timesheet_is_corrected_by_an_adjustment_and_left_alone() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    let original = eight_hours(&t, a, TimesheetStatus::Locked).await;
    let before = t.reload_sheet(&original).await;

    adjust(
        &t,
        &a_user,
        &original,
        &[
            ("reason", "marked present, was absent"),
            ("origin", "attendance_error"),
            ("affects", "both"),
            ("entries", entries("normal", -480).as_str()),
        ],
    )
    .await
    .assert_ok();

    let list = adjustments_of(&t, &original).await;
    assert_eq!(list.len(), 1);
    let adjustment = &list[0];
    assert_eq!(adjustment.kind, TimesheetKind::Adjustment);
    assert_eq!(adjustment.status, TimesheetStatus::Open);
    assert_eq!(adjustment.adjusts_id, Some(original.id));
    assert_eq!(adjustment.reason.as_deref(), Some("marked present, was absent"));
    assert_eq!(adjustment.origin, Some(AdjustmentOrigin::AttendanceError));
    assert_eq!(adjustment.affects, Some(Affects::Both));
    assert_eq!(adjustment.created_by, Some(a_user.id));
    assert_eq!(adjustment.owner(), a);
    assert_eq!(adjustment.context(), Some(project(5)));
    assert_eq!(
        (adjustment.period_from, adjustment.period_to),
        (nov(2), nov(8))
    );
    assert_eq!(adjustment.approver_kind, original.approver_kind);
    assert!(!adjustment.consumed());
    let rows = TimesheetEntry::of_timesheet(t.db(), adjustment.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].normal_minutes, -480);
    assert_eq!(rows[0].date, nov(2));
    assert_eq!(rows[0].cost_object(), Some(cost_code()));

    // The original is as it was: state, consumers, entries, even its stamp.
    let after = t.reload_sheet(&original).await;
    assert_eq!(after.status, TimesheetStatus::Locked);
    assert_eq!(after.paid_by.as_deref(), Some("hr.payroll_run:7"));
    assert_eq!(after.updated_at, before.updated_at);
    let kept = TimesheetEntry::of_timesheet(t.db(), original.id).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].normal_minutes, 480);

    // `reopen` is refused on a locked timesheet.
    t.app.acting_as(&keeper);
    t.app
        .htmx()
        .post(
            &format!("/timesheets/{}/reopen", original.id),
            &[("reason", "wrong hours")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.reload_sheet(&original).await.status, TimesheetStatus::Locked);
}

/// Several adjustments on one original are allowed, each linked.
#[renox::test]
async fn att07_an_original_may_have_several_adjustments() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let a = t.employee(1, &[]);
    let original = eight_hours(&t, a, TimesheetStatus::Locked).await;
    for minutes in [-60, 120] {
        adjust(
            &t,
            &a_user,
            &original,
            &[
                ("reason", "rework"),
                ("origin", "payroll_error"),
                ("affects", "payroll"),
                ("entries", entries("normal", minutes).as_str()),
            ],
        )
        .await
        .assert_ok();
    }
    assert_eq!(adjustments_of(&t, &original).await.len(), 2);
}

/// An `approved` timesheet that is not locked is not corrected by an
/// adjustment (it is reopened), and neither is an adjustment.
#[renox::test]
async fn att07_only_a_locked_regular_timesheet_takes_an_adjustment() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let a = t.employee(1, &[]);
    let form = [
        ("reason", "typo"),
        ("origin", "attendance_error"),
        ("affects", "both"),
    ];
    let entries_json = entries("normal", -60);
    let mut with_entries = form.to_vec();
    with_entries.push(("entries", entries_json.as_str()));

    let approved = eight_hours(&t, a, TimesheetStatus::Approved).await;
    adjust(&t, &a_user, &approved, &with_entries)
        .await
        .assert_status(422)
        .assert_see("reopened instead");
    let open = eight_hours(&t, a, TimesheetStatus::Open).await;
    adjust(&t, &a_user, &open, &with_entries)
        .await
        .assert_status(422);
    assert!(adjustments_of(&t, &approved).await.is_empty());
    assert!(adjustments_of(&t, &open).await.is_empty());

    // An adjustment never points to another adjustment, locked or not.
    let locked = eight_hours(&t, a, TimesheetStatus::Locked).await;
    adjust(&t, &a_user, &locked, &with_entries).await.assert_ok();
    let inner = adjustments_of(&t, &locked).await.remove(0);
    let mut stand_in = inner.clone();
    stand_in.id = 0;
    let locked_adjustment = t.sheet_in(stand_in, TimesheetStatus::Locked).await;
    adjust(&t, &a_user, &locked_adjustment, &with_entries)
        .await
        .assert_status(422)
        .assert_see("never to another adjustment");
    assert!(adjustments_of(&t, &locked_adjustment).await.is_empty());
}

/// A reason and an origin are required; a client dispute needs the linked
/// ticket or dispute and then defaults to billing; the others need `affects`;
/// at least one entry; nothing is written by a refusal.
#[renox::test]
async fn att07_the_reason_origin_link_and_entries_are_required() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let a = t.employee(1, &[]);
    let original = eight_hours(&t, a, TimesheetStatus::Locked).await;
    let e = entries("normal", -60);

    for form in [
        // No reason.
        vec![("origin", "attendance_error"), ("affects", "both"), ("entries", e.as_str())],
        vec![("reason", "  "), ("origin", "attendance_error"), ("affects", "both"), ("entries", e.as_str())],
        // No origin, or an unknown one.
        vec![("reason", "x"), ("affects", "both"), ("entries", e.as_str())],
        vec![("reason", "x"), ("origin", "mood"), ("affects", "both"), ("entries", e.as_str())],
        // A client dispute with nothing linked.
        vec![("reason", "x"), ("origin", "client_dispute"), ("entries", e.as_str())],
        // No `affects` for an origin with no default.
        vec![("reason", "x"), ("origin", "payroll_error"), ("entries", e.as_str())],
        // No entries, or one with no hours.
        vec![("reason", "x"), ("origin", "payroll_error"), ("affects", "payroll")],
        vec![
            ("reason", "x"),
            ("origin", "payroll_error"),
            ("affects", "payroll"),
            ("entries", "[]"),
        ],
        // Entries that cannot be read.
        vec![("reason", "x"), ("origin", "payroll_error"), ("affects", "payroll"), ("entries", "oops")],
    ] {
        adjust(&t, &a_user, &original, &form).await.assert_status(422);
    }
    assert!(adjustments_of(&t, &original).await.is_empty());

    // A client dispute with its link: billing by default.
    adjust(
        &t,
        &a_user,
        &original,
        &[
            ("reason", "client disputed 2 hours"),
            ("origin", "client_dispute"),
            ("linked", "invoicing.invoice:12"),
            ("entries", e.as_str()),
        ],
    )
    .await
    .assert_ok();
    let list = adjustments_of(&t, &original).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].affects, Some(Affects::Billing));
    assert_eq!(list[0].linked(), Some(Subject::new("invoicing.invoice", 12)));
    assert_eq!(list[0].origin, Some(AdjustmentOrigin::ClientDispute));
}

/// A -10 h entry on a day where T has 8 h is refused; so is one that, with the
/// approved adjustments, takes the day below zero; and the entries of one
/// request add up.
#[renox::test]
async fn att07_a_delta_cannot_take_the_day_below_zero() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let a = t.employee(1, &[]);
    let original = eight_hours(&t, a, TimesheetStatus::Locked).await;
    let post = |json: String| adjust_json(&t, &a_user, &original, json);

    post(entries("normal", -600)).await.assert_status(422);
    // Another category has no hours to take away.
    post(entries("overtime", -60)).await.assert_status(422);
    // Two entries of one request: 5 h and 4 h out of 8.
    let two = format!(
        r#"[{{"date": "2026-11-02", "category": "normal", "minutes": -300}},
            {{"date": "2026-11-02", "category": "normal", "minutes": -240}}]"#
    );
    post(two).await.assert_status(422);
    assert!(adjustments_of(&t, &original).await.is_empty());
    // A day outside the original's period.
    post(r#"[{"date": "2026-11-20", "category": "normal", "minutes": 60}]"#.to_owned())
        .await
        .assert_status(422);

    // 8 h out is fine; added hours too.
    post(entries("normal", -480)).await.assert_ok();
    post(entries("normal", 90)).await.assert_ok();

    // An approved adjustment of -8 h leaves nothing to take away.
    let approved = Timesheet {
        owner_employee_id: a.0,
        period_from: nov(2),
        period_to: nov(8),
        kind: TimesheetKind::Adjustment,
        adjusts_id: Some(original.id),
        reason: Some("earlier".into()),
        origin: Some(AdjustmentOrigin::PayrollError),
        affects: Some(Affects::Payroll),
        ..Timesheet::default()
    };
    let approved = t.sheet_in(approved, TimesheetStatus::Approved).await;
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: approved.id,
            employee_id: a.0,
            date: nov(2),
            normal_minutes: -480,
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
    post(entries("normal", -60)).await.assert_status(422);
    // The standing hours are 0 now, but +1 h is still allowed.
    post(entries("normal", 60)).await.assert_ok();
}

/// Only `timesheets.adjust` opens one.
#[renox::test]
async fn att07_opening_an_adjustment_needs_the_permission() {
    let t = boot().await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    let original = eight_hours(&t, a, TimesheetStatus::Locked).await;
    adjust(
        &t,
        &keeper,
        &original,
        &[
            ("reason", "x"),
            ("origin", "payroll_error"),
            ("affects", "payroll"),
            ("entries", entries("normal", -60).as_str()),
        ],
    )
    .await
    .assert_forbidden();
    assert!(adjustments_of(&t, &original).await.is_empty());
}

/// An adjustment is left out of the sum of `submit` (BL-ATT-06), and its own
/// submit runs no attendance check.
#[renox::test]
async fn att07_an_adjustment_does_not_count_for_the_attended_hours() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    t.fixture(a, &site, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Accepted)
        .await;
    // T locked with 5 h; an adjustment of +4 h (more than the 3 h left).
    let mut new = Timesheet::regular(a, nov(2), nov(8)).in_context(&project(5));
    new.paid_by = Some("hr.payroll_run:7".into());
    let original = t.sheet_in(new, TimesheetStatus::Locked).await;
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: original.id,
            employee_id: a.0,
            date: nov(2),
            cost_object_type: Some(project(5).subject_type),
            cost_object_id: Some(5),
            normal_minutes: 300,
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
    adjust(
        &t,
        &a_user,
        &original,
        &[
            ("reason", "extra work"),
            ("origin", "attendance_error"),
            ("affects", "both"),
            ("entries", entries("normal", 240).as_str()),
        ],
    )
    .await
    .assert_ok();
    let adjustment = adjustments_of(&t, &original).await.remove(0);

    // The adjustment submits with no attendance check.
    t.app.acting_as(&keeper);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/submit", adjustment.id), &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(
        t.reload_sheet(&adjustment).await.status,
        TimesheetStatus::Submitted
    );

    // A regular timesheet of 3 h on the same day is within 8 h (5 + 3).
    let regular = Timesheet::create(
        t.db(),
        Timesheet::regular(a, nov(2), nov(8)).in_context(&project(6)),
    )
    .await
    .unwrap();
    entry::add(
        t.state(),
        regular.id,
        NewEntry::of(nov(2), Category::Normal, 180)
            .on(&project(6))
            .from_attendance(),
    )
    .await
    .unwrap();
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/submit", regular.id), &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(
        t.reload_sheet(&regular).await.status,
        TimesheetStatus::Submitted
    );
}
