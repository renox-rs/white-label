//! BL-HRM-16 (#206) with the real wl-attendance in place of the fake
//! `Timesheets`: the provisional hours the module gives payroll, the
//! `paid_provisionally_by` that `submit` leaves on the timesheet, and the
//! reconciliation event the module emits when the client approves fewer hours
//! than were paid (wl-hr §5.3, wl-attendance §5.7 points 1 and 3).

mod support;

use renox::auth::permissions;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_attendance::timesheet::ApproverKind;
use wl_attendance::{
    AttendanceMethod, AttendanceRecord, AttendanceStatus, Timesheet, TimesheetEntry,
    TimesheetStatus, WorkLocation,
};
use wl_hr::{Employee, EmployeeStatus, HourDifference, PayrollRun, PayrollRunStatus};
use wl_kernel::BusinessCalendar;
use wl_kernel::Subject;

use support::{Booted, nov};

/// A wage of AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// AED `dirhams` in fils.
fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

/// `hour`:00 on `day` November 2026 in Asia/Dubai.
fn at(day: u32, hour: u32) -> DateTime {
    BusinessCalendar::default().from_local(nov(day).and_hms_opt(hour, 0, 0).unwrap())
}

/// Employee T, a preparer, an approver of payroll, the client's approver, the
/// November run and T's submitted timesheet with 160 accepted attendance
/// hours (2 to 21 November), whose client has not approved it yet.
struct Desk {
    t: Booted,
    prep: User,
    appr: User,
    client: User,
    e: Employee,
    november: PayrollRun,
    sheet: Timesheet,
}

async fn desk() -> Desk {
    let t = support::boot_with_attendance().await;
    permissions::define_role(t.db(), "client_approver", &[])
        .await
        .unwrap();
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    let client = t.user_with("Carl", "client_approver").await;
    let e = t.employee_in(71, EmployeeStatus::Active).await;
    t.salary(&e, WAGE, aed(15)).await;
    let november = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    let mut new = Timesheet::regular(e.employee_id(), nov(1), nov(30))
        .in_context(&Subject::new("manpower.deployment", 9));
    new.approver_kind = ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    new.status = TimesheetStatus::Submitted;
    let mut sheet = new;
    let mut tx = t.db().begin().await.unwrap();
    sheet
        .insert_in_state(&mut tx, TimesheetStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let site = WorkLocation::create(t.db(), WorkLocation::named("Site 71"))
        .await
        .unwrap();
    for day in 2..=21 {
        let mut record = AttendanceRecord {
            employee_id: e.id,
            location_id: site.id,
            date: nov(day),
            check_in: at(day, 7),
            check_out: Some(at(day, 15)),
            method: AttendanceMethod::Supervisor,
            recorded_at: at(day, 7),
            status: AttendanceStatus::Accepted,
            ..AttendanceRecord::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        record
            .insert_in_state(&mut tx, AttendanceStatus::Accepted, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    Desk {
        t,
        prep,
        appr,
        client,
        e,
        november,
        sheet,
    }
}

impl Desk {
    /// Calculates, approves and submits the November run.
    async fn pay_november(&self) {
        self.t
            .transition(&self.prep, &self.november, "calculate", "")
            .await
            .assert_status(303);
        self.t
            .transition(&self.appr, &self.november, "approve", "")
            .await
            .assert_status(303);
        wl_hr::payroll::submit(self.t.state(), self.november.id)
            .await
            .unwrap();
    }

    async fn sheet(&self) -> Timesheet {
        Timesheet::find_or_404(self.t.db(), self.sheet.id)
            .await
            .unwrap()
    }
}

/// AC1: employee T's timesheet is provisional with 160 h: the line shows 160 h
/// provisional, and on `submit` the real timesheet gets `paid_provisionally_by`
/// = the November run.
#[renox::test]
async fn hrm16_ac1_submit_marks_the_real_timesheet_paid_provisionally_by_the_run() {
    let d = desk().await;
    d.t.transition(&d.prep, &d.november, "calculate", "")
        .await
        .assert_status(303);
    let line = d.t.line_of(&d.november, &d.e).await;
    assert_eq!(
        (line.hours_minutes, line.provisional_minutes),
        (9_600, 9_600)
    );
    // Nothing is told to the timesheet before the file is sent.
    assert_eq!(d.sheet().await.paid_provisionally_by, None);

    d.t.transition(&d.appr, &d.november, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(d.t.state(), d.november.id)
        .await
        .unwrap();

    let sheet = d.sheet().await;
    assert_eq!(
        sheet.paid_provisionally_by,
        Some(d.november.subject().to_string())
    );
    assert_eq!(sheet.paid_by, None, "provisional hours are not final");
}

/// AC1 and AC3, end to end: the client approves 152 h of the 160 h paid
/// provisionally; the real module emits `TimesheetReconciliationNeeded` (no
/// hand-emitted event), and the December run recovers the -8 h at AED 15/h.
#[renox::test]
async fn hrm16_ac1_the_reconciliation_event_of_the_real_module_reaches_the_next_run() {
    let d = desk().await;
    d.pay_november().await;
    for day in 2..=20 {
        TimesheetEntry::create(
            d.t.db(),
            TimesheetEntry {
                timesheet_id: d.sheet.id,
                employee_id: d.e.id,
                date: nov(day),
                cost_object_type: Some("manpower.deployment".into()),
                cost_object_id: Some(9),
                normal_minutes: 480,
                billable: true,
                ..TimesheetEntry::default()
            },
        )
        .await
        .unwrap();
    }

    d.t.app.acting_as(&d.client);
    d.t.app
        .htmx()
        .post(
            &format!("/timesheets/{}/approve", d.sheet.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();

    let adjustments = d.sheet().await.adjusted_by(d.t.db()).await.unwrap();
    assert_eq!(adjustments.len(), 1);
    let differences = HourDifference::query()
        .where_(HourDifference::EMPLOYEE_ID.eq(d.e.id))
        .get(d.t.db())
        .await
        .unwrap();
    assert_eq!(differences.len(), 1);
    assert_eq!(
        differences[0].source,
        wl_hr::differences::SOURCE_RECONCILIATION
    );

    let december = d.t.payroll_run("2026-12", PayrollRunStatus::Draft).await;
    d.t.transition(&d.prep, &december, "calculate", "")
        .await
        .assert_status(303);
    let line = d.t.line_of(&december, &d.e).await;
    assert_eq!(line.difference_amount, -aed(120));
    assert_eq!(line.net_amount, WAGE - aed(120));
    // The event the module emitted names the run that paid the hours.
    assert_eq!(
        adjustments[0].linked(),
        Some(Subject::new("hr.payroll_run", d.november.id))
    );
}
