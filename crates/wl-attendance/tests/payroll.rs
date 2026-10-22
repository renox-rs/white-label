//! BL-ATT-12 (#189): `Timesheets::hours_for_payroll` gives payroll the
//! approved hours and, for timesheets the client has not approved, the
//! accepted attendance hours marked provisional (wl-attendance §5.7 points 1
//! and 2, §6 `Timesheets`, §7 *Payroll basis*).

mod support;

use renox::prelude::*;
use wl_attendance::config::PayrollBasis;
use wl_attendance::payroll::ProvisionalHours;
use wl_attendance::timesheet::ApproverKind;
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetEntry, TimesheetStatus, WlAttendance};
use wl_kernel::Subject;
use wl_kernel::attendance::CategoryMinutes;
use wl_kernel::timesheets::{PayrollHours, PayrollPeriod, timesheets};
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, nov, post_to};

fn november() -> PayrollPeriod {
    PayrollPeriod::new(nov(1), nov(30), nov(25))
}

fn run() -> Subject {
    Subject::new("hr.payroll_run", 7)
}

fn deployment() -> Subject {
    Subject::new("manpower.deployment", 9)
}

async fn hours(t: &Booted) -> Vec<PayrollHours> {
    timesheets(t.state())
        .hours_for_payroll(t.state(), &november())
        .await
        .unwrap()
}

/// An 8 h accepted record (07:00 to 15:00) of `employee` on each of `days`
/// of November, in `status`.
async fn attended(t: &Booted, employee: EmployeeId, days: &[u32], status: AttendanceStatus) {
    let site = t.location(&format!("Site {}", employee.0)).await;
    for day in days {
        t.fixture(
            employee,
            &site,
            at(*day, 7, 0),
            Some(at(*day, 15, 0)),
            status,
        )
        .await;
    }
}

/// The twenty working days of the examples: 2 to 21 November, 160 h.
fn twenty_days() -> Vec<u32> {
    (2..=21).collect()
}

/// `minutes` of normal hours on each of `days`, as entries of `sheet`.
async fn entries(t: &Booted, sheet: &Timesheet, days: &[u32], minutes: i64) {
    for day in days {
        TimesheetEntry::create(
            t.db(),
            TimesheetEntry {
                timesheet_id: sheet.id,
                employee_id: sheet.owner_employee_id,
                date: nov(*day),
                cost_object_type: Some("manpower.deployment".into()),
                cost_object_id: Some(9),
                normal_minutes: minutes,
                billable: true,
                ..TimesheetEntry::default()
            },
        )
        .await
        .unwrap();
    }
}

/// A November timesheet for the deployment whose client approves it, in
/// `status`.
async fn client_sheet(t: &Booted, employee: EmployeeId, status: TimesheetStatus) -> Timesheet {
    let mut new = Timesheet::regular(employee, nov(1), nov(30)).in_context(&deployment());
    new.approver_kind = ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    t.sheet_in(new, status).await
}

fn normal(minutes: i64) -> Vec<CategoryMinutes> {
    vec![CategoryMinutes {
        category: "normal".into(),
        minutes,
    }]
}

/// AC1 and AC3: given T (client approver) `submitted` with 160 accepted
/// attendance hours for November and the cut-off on 2026-11-25,
/// `hours_for_payroll` returns 160 h marked provisional; an 8 h record still
/// `flagged` at the cut-off is left out (160 h, not 168 h), and so is
/// attendance dated after the cut-off.
#[renox::test]
async fn att12_a_submitted_timesheet_gives_its_accepted_attendance_as_provisional_hours() {
    let t = boot().await;
    let worker = t.employee(1, &[]);
    let sheet = client_sheet(&t, worker, TimesheetStatus::Submitted).await;
    attended(&t, worker, &twenty_days(), AttendanceStatus::Accepted).await;
    // Flagged at the cut-off, and recorded after it.
    attended(&t, worker, &[23], AttendanceStatus::Flagged).await;
    attended(&t, worker, &[26], AttendanceStatus::Accepted).await;

    let found = hours(&t).await;
    assert_eq!(found.len(), 1);
    let got = &found[0];
    assert_eq!(got.timesheet, sheet.subject());
    assert!(got.provisional);
    assert_eq!(got.total_minutes(), 160 * 60);
    assert_eq!(got.totals(), normal(160 * 60));
    assert_eq!(got.owner, worker);
    assert_eq!(got.approver_kind, "external");
    assert_eq!(got.context, Some(deployment()));
    assert_eq!(got.lines.len(), 20);
    assert_eq!((got.lines[0].date, got.lines[0].minutes), (nov(2), 480));

    // No side effect, and the same answer until the data changes.
    assert_eq!(hours(&t).await, found);
    let stored = t.reload_sheet(&sheet).await;
    assert_eq!(stored.status, TimesheetStatus::Submitted);
    assert!(!stored.consumed());
    assert!(
        ProvisionalHours::of_timesheet(t.db(), sheet.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC2: an `approved` timesheet U with 150 h gives 150 h, not provisional;
/// an `open` one gives provisional hours too; one used already, one whose
/// last day is in another period, hours entered by hand and an adjustment are
/// not offered.
#[renox::test]
async fn att12_an_approved_timesheet_gives_its_own_hours_and_the_others_stay_out() {
    let t = boot().await;
    let u_owner = t.employee(1, &[]);
    let u = client_sheet(&t, u_owner, TimesheetStatus::Approved).await;
    // 150 h: 25 days of 6 h.
    entries(&t, &u, &(1..=25).collect::<Vec<_>>(), 360).await;

    let open_owner = t.employee(2, &[]);
    let open = t
        .sheet_in(
            Timesheet::regular(open_owner, nov(1), nov(30)),
            TimesheetStatus::Open,
        )
        .await;
    attended(&t, open_owner, &[2, 3], AttendanceStatus::Accepted).await;

    // Not offered: used by a run, another period, entered by hand, an
    // adjustment, and one with no accepted attendance.
    let paid_owner = t.employee(3, &[]);
    let mut paid = Timesheet::regular(paid_owner, nov(1), nov(30));
    paid.paid_by = Some("hr.payroll_run:6".into());
    let paid = t.sheet_in(paid, TimesheetStatus::Locked).await;
    entries(&t, &paid, &[2], 480).await;
    let other_owner = t.employee(4, &[]);
    let december = t
        .sheet_in(
            Timesheet::regular(other_owner, nov(30), nov(30) + renox::chrono::Days::new(6)),
            TimesheetStatus::Approved,
        )
        .await;
    entries(&t, &december, &[30], 480).await;
    let manual_owner = t.employee(5, &[]);
    let manual = Timesheet::regular(manual_owner, nov(1), nov(30))
        .from_source(wl_attendance::timesheet::HoursSource::Manual);
    t.sheet_in(manual, TimesheetStatus::Submitted).await;
    attended(&t, manual_owner, &[2], AttendanceStatus::Accepted).await;
    let empty_owner = t.employee(6, &[]);
    t.sheet_in(
        Timesheet::regular(empty_owner, nov(1), nov(30)),
        TimesheetStatus::Open,
    )
    .await;

    let found = hours(&t).await;
    let subjects: Vec<Subject> = found.iter().map(|h| h.timesheet.clone()).collect();
    assert_eq!(subjects, vec![u.subject(), open.subject()]);
    assert!(!found[0].provisional);
    assert_eq!(found[0].total_minutes(), 150 * 60);
    assert_eq!(found[0].lines.len(), 25);
    assert_eq!(found[0].approver_kind, "external");
    assert!(found[1].provisional);
    assert_eq!(found[1].total_minutes(), 16 * 60);
    assert_eq!(found[1].approver_kind, "internal");
}

/// AC4: when the payroll run is submitted, T has `paid_provisionally_by` =
/// the run (and the hours paid are kept) and U is `locked` with `paid_by` =
/// the run; told again, nothing changes, and neither is offered again.
#[renox::test]
async fn att12_the_submitted_run_marks_the_timesheets_it_used() {
    let t = boot().await;
    let worker = t.employee(1, &[]);
    let sheet = client_sheet(&t, worker, TimesheetStatus::Submitted).await;
    attended(&t, worker, &twenty_days(), AttendanceStatus::Accepted).await;
    let u_owner = t.employee(2, &[]);
    let u = client_sheet(&t, u_owner, TimesheetStatus::Approved).await;
    entries(&t, &u, &(1..=25).collect::<Vec<_>>(), 360).await;

    // The run is calculated: nothing is marked by asking.
    let used = hours(&t).await;
    assert_eq!(used.len(), 2);
    assert_eq!(hours(&t).await, used);
    assert_eq!(t.reload_sheet(&u).await.status, TimesheetStatus::Approved);

    // The run is submitted.
    let service = timesheets(t.state());
    service
        .record_payroll_use(t.state(), &run(), &used)
        .await
        .unwrap();
    service
        .record_payroll_use(t.state(), &run(), &used)
        .await
        .unwrap();

    let t_after = t.reload_sheet(&sheet).await;
    assert_eq!(
        t_after.paid_provisionally_by.as_deref(),
        Some("hr.payroll_run:7")
    );
    assert_eq!(t_after.paid_by, None);
    assert_eq!(t_after.status, TimesheetStatus::Submitted);
    let kept = ProvisionalHours::of_timesheet(t.db(), sheet.id)
        .await
        .unwrap();
    assert_eq!(kept.len(), 20, "kept once, not once per call");
    assert_eq!(kept.iter().map(|h| h.minutes).sum::<i64>(), 160 * 60);
    assert!(kept.iter().all(|h| h.payroll_run == "hr.payroll_run:7"));
    assert!(kept.iter().all(|h| h.category == "normal"));

    let u_after = t.reload_sheet(&u).await;
    assert_eq!(u_after.status, TimesheetStatus::Locked);
    assert_eq!(u_after.paid_by.as_deref(), Some("hr.payroll_run:7"));
    assert_eq!(u_after.paid_provisionally_by, None);

    assert!(hours(&t).await.is_empty(), "neither is paid twice");
}

/// A run that was calculated but not submitted (returned or cancelled) marks
/// nothing; a timesheet another run paid is refused for this run.
#[renox::test]
async fn att12_only_a_submitted_run_marks_and_a_paid_timesheet_is_not_paid_twice() {
    let t = boot().await;
    let worker = t.employee(1, &[]);
    let sheet = client_sheet(&t, worker, TimesheetStatus::Submitted).await;
    attended(&t, worker, &[2, 3], AttendanceStatus::Accepted).await;
    let first = hours(&t).await;
    // Calculated again after a return: the same, nothing marked.
    assert_eq!(hours(&t).await, first);
    assert_eq!(t.reload_sheet(&sheet).await.paid_provisionally_by, None);

    let service = timesheets(t.state());
    service
        .record_payroll_use(t.state(), &run(), &first)
        .await
        .unwrap();
    let other = Subject::new("hr.payroll_run", 8);
    assert!(
        service
            .record_payroll_use(t.state(), &other, &first)
            .await
            .is_err()
    );
    assert_eq!(
        t.reload_sheet(&sheet)
            .await
            .paid_provisionally_by
            .as_deref(),
        Some("hr.payroll_run:7")
    );
    // Something that is not a timesheet.
    let mut stray = first.clone();
    stray[0].timesheet = Subject::new("hr.employee", 1);
    assert!(
        service
            .record_payroll_use(t.state(), &run(), &stray)
            .await
            .is_err()
    );
}

/// AC5: when T is later approved, it moves to `locked` in the same step with
/// `paid_provisionally_by` = the run and no `paid_by`; `reopen` on it is
/// refused. An approved timesheet that carries the mark is refused too, with
/// its own reason.
#[renox::test]
async fn att12_a_provisionally_paid_timesheet_locks_when_approved_and_is_not_reopened() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let worker = t.employee(1, &[]);
    let sheet = client_sheet(&t, worker, TimesheetStatus::Submitted).await;
    attended(&t, worker, &twenty_days(), AttendanceStatus::Accepted).await;
    // The hours the client will approve are the 160 h that were paid.
    entries(&t, &sheet, &twenty_days(), 480).await;
    let used = hours(&t).await;
    timesheets(t.state())
        .record_payroll_use(t.state(), &run(), &used)
        .await
        .unwrap();

    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();
    let locked = t.reload_sheet(&sheet).await;
    assert_eq!(locked.status, TimesheetStatus::Locked);
    assert_eq!(
        locked.paid_provisionally_by.as_deref(),
        Some("hr.payroll_run:7")
    );
    assert_eq!(locked.paid_by, None);
    assert!(locked.consumed());
    assert_eq!(t.approved.all().len(), 1, "the approval is announced once");
    let history = t.sheet_history(&sheet).await;
    let steps: Vec<&str> = history.iter().map(|h| h.0.as_str()).collect();
    assert!(steps.ends_with(&["approve", "lock"]), "{steps:?}");

    post_to(&t, &keeper, &sheet, "reopen", &[("reason", "typo")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_sheet(&sheet).await.status, TimesheetStatus::Locked);

    // Approved and marked (not yet locked): the reason is the provisional pay.
    let owner = t.employee(2, &[]);
    let mut marked = Timesheet::regular(owner, nov(1), nov(30));
    marked.paid_provisionally_by = Some("hr.payroll_run:7".into());
    let marked = t.sheet_in(marked, TimesheetStatus::Approved).await;
    post_to(&t, &keeper, &marked, "reopen", &[("reason", "typo")])
        .await
        .assert_status(422)
        .assert_see("provisional hours");
}

/// With the payroll basis `wait` on an internal context, unapproved T gives 0
/// h; an external approver's timesheet is still paid on attendance. Setting
/// `wait` for an external approver is refused in the settings.
#[renox::test]
async fn att12_wait_holds_back_an_internal_timesheet_and_is_refused_for_an_external_one() {
    let module = WlAttendance::new()
        .payroll_basis(ApproverKind::Internal, PayrollBasis::Wait)
        .unwrap();
    assert_eq!(
        module.config().payroll_basis_for(ApproverKind::Internal),
        PayrollBasis::Wait
    );
    let t = support::boot_with(module, "").await;
    let a = t.employee(1, &[]);
    let internal = t
        .sheet_in(
            Timesheet::regular(a, nov(1), nov(30)).in_context(&deployment()),
            TimesheetStatus::Submitted,
        )
        .await;
    attended(&t, a, &twenty_days(), AttendanceStatus::Accepted).await;
    let b = t.employee(2, &[]);
    let external = client_sheet(&t, b, TimesheetStatus::Submitted).await;
    attended(&t, b, &[2], AttendanceStatus::Accepted).await;

    let found = hours(&t).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].timesheet, external.subject());
    assert!(found[0].provisional);
    assert_eq!(found[0].total_minutes(), 8 * 60);
    assert_eq!(t.reload_sheet(&internal).await.paid_provisionally_by, None);

    // The settings refuse `wait` for an external approver; the attendance
    // basis is always fine; the default basis is attendance.
    let refused = WlAttendance::new()
        .payroll_basis(ApproverKind::External, PayrollBasis::Wait)
        .err()
        .expect("wait is refused for an external approver");
    assert!(refused.to_string().contains("employer's own staff"));
    assert!(
        WlAttendance::new()
            .payroll_basis(ApproverKind::External, PayrollBasis::Attendance)
            .is_ok()
    );
    assert_eq!(
        WlAttendance::new()
            .config()
            .payroll_basis_for(ApproverKind::Internal),
        PayrollBasis::Attendance
    );
}
