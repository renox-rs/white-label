//! BL-HRM-16 (#206), `calculate` and `submit` of a regular payroll run (wl-hr
//! §4 *Payroll run*, §5.3 step 2 and 3): the lines come from the salary
//! structures and the hours of the timesheets, provisional hours are shown as
//! such, an employee without hours is a listed exception, and `submit` tells
//! the timesheets which hours were paid. Also `HRM.ACL.04` and `HRM.ACL.06`
//! on the calculated lines.

mod support;

use renox::prelude::*;
use wl_hr::{Employee, EmployeeStatus, EmploymentType, LineStatus, PayrollRunStatus};
use wl_kernel::Subject;
use wl_kernel::workforce::AbsenceCaseClosed;

use support::{Booted, hours_of};

/// A wage of AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// AED `dirhams` in fils.
fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

/// A preparer and an approver.
struct Desk {
    t: Booted,
    prep: User,
    appr: User,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    Desk { t, prep, appr }
}

/// AC1: employee T's timesheet is provisional with 160 h: the November line
/// shows 160 h marked provisional; on `submit` the timesheet is told the
/// November run paid it, so that it gets `paid_provisionally_by`.
#[renox::test]
async fn hrm16_ac1_provisional_hours_are_marked_on_the_line_and_submit_tells_the_timesheet() {
    let d = desk().await;
    let t_emp = d.t.employee_in(71, EmployeeStatus::Active).await;
    d.t.salary(&t_emp, WAGE, aed(15)).await;
    let hours = hours_of(&t_emp, 3, true, &[(2, "normal", 9_600)]);
    d.t.timesheets.give(hours.clone());
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);

    let line = d.t.line_of(&run, &t_emp).await;
    assert_eq!(
        (line.hours_minutes, line.provisional_minutes),
        (9_600, 9_600)
    );
    d.t.app.acting_as(&d.prep);
    d.t.app
        .get(&format!("/hr/payroll/{}", run.id))
        .await
        .assert_ok()
        .assert_see("160 h")
        .assert_see("provisional");

    // Nothing is told to the timesheets before the file is sent.
    assert!(d.t.timesheets.used().is_empty());
    d.t.transition(&d.appr, &run, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(d.t.state(), run.id).await.unwrap();

    assert_eq!(d.t.timesheets.used(), vec![(run.subject(), vec![hours])]);
}

/// AC1, the other side: hours of a line the file left out (held) are not told
/// as paid.
#[renox::test]
async fn hrm16_ac1_the_hours_of_a_held_line_are_not_told_as_paid() {
    let d = desk().await;
    let ok = d.t.employee_in(71, EmployeeStatus::Active).await;
    let away = d.t.employee_in(72, EmployeeStatus::Active).await;
    for e in [&ok, &away] {
        d.t.salary(e, WAGE, aed(15)).await;
    }
    d.t.timesheets
        .give(hours_of(&ok, 3, false, &[(2, "normal", 9_600)]));
    d.t.timesheets
        .give(hours_of(&away, 4, false, &[(2, "normal", 9_600)]));
    d.t.make_absent(&away).await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.appr, &run, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(d.t.state(), run.id).await.unwrap();

    let used = d.t.timesheets.used();
    assert_eq!(used.len(), 1);
    let told: Vec<Subject> = used[0].1.iter().map(|h| h.timesheet.clone()).collect();
    assert_eq!(told, vec![Subject::new("attendance.timesheet", 3)]);
}

/// AC2: employee G has no timesheet: `calculate` succeeds and G's line is
/// listed as a *missing timesheet* exception.
#[renox::test]
async fn hrm16_ac2_an_employee_without_a_timesheet_is_a_listed_exception_not_a_block() {
    let d = desk().await;
    let g = d.t.employee_in(73, EmployeeStatus::Active).await;
    d.t.salary(&g, WAGE, aed(15)).await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);

    assert_eq!(d.t.run(&run).await.status, PayrollRunStatus::Calculated);
    let line = d.t.line_of(&run, &g).await;
    assert_eq!(line.exception.as_deref(), Some("missing_timesheet"));
    assert_eq!(line.status, LineStatus::Included);
    d.t.app.acting_as(&d.prep);
    d.t.app
        .get(&format!("/hr/payroll/{}", run.id))
        .await
        .assert_ok()
        .assert_see("Missing timesheet");
}

/// The line is filled from the salary structure: its `fixed_wage` is the
/// basis of the Art. 25 caps, and overtime hours add their pay at 125% of the
/// hourly rate.
#[renox::test]
async fn hrm16_the_line_takes_the_fixed_wage_from_the_structure_and_pays_overtime() {
    let d = desk().await;
    let e = d.t.employee_in(74, EmployeeStatus::Active).await;
    d.t.salary(&e, WAGE, aed(15)).await;
    d.t.timesheets.give(hours_of(
        &e,
        5,
        false,
        &[(2, "normal", 480), (3, "overtime", 240)],
    ));
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);

    let line = d.t.line_of(&run, &e).await;
    assert_eq!(line.fixed_wage, WAGE);
    // 4 h of overtime at 125% of AED 15 an hour.
    assert_eq!(line.variable_amount, aed(75));
    assert_eq!(line.net_amount, WAGE + aed(75));
    assert_eq!(line.exception, None);
}

/// The cap check reads the `fixed_wage` the calculation filled in: a recovery
/// of AED 700 on a wage of AED 3,000 refuses `approve`.
#[renox::test]
async fn hrm16_the_caps_read_the_fixed_wage_the_calculation_filled() {
    let d = desk().await;
    let e = d.t.employee_in(75, EmployeeStatus::Active).await;
    d.t.salary(&e, WAGE, aed(15)).await;
    d.t.deduct(
        &e,
        wl_hr::AdjustmentKind::OverpaymentRecovery,
        aed(700),
        "2026-11",
    )
    .await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);

    let line = d.t.line_of(&run, &e).await;
    assert_eq!((line.fixed_wage, line.deductions_amount), (WAGE, aed(700)));
    assert_eq!(line.net_amount, WAGE - aed(700));
    d.t.transition(&d.appr, &run, "approve", "")
        .await
        .assert_status(422);
}

/// A freelancer has no payroll line, and an employee the calculation does not
/// know (no salary structure) keeps the line the run holds.
#[renox::test]
async fn hrm16_a_freelancer_gets_no_line_and_a_line_without_a_structure_stays_as_entered() {
    let d = desk().await;
    let freelancer = d.t.employee_in(76, EmployeeStatus::Active).await;
    d.t.salary(&freelancer, WAGE, aed(15)).await;
    Employee::query()
        .where_(Employee::ID.eq(freelancer.id))
        .update(
            d.t.db(),
            &[(
                "employment_type",
                &EmploymentType::Freelance.as_str().to_owned(),
            )],
        )
        .await
        .unwrap();
    let manual = d.t.employee_in(77, EmployeeStatus::Active).await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    d.t.payroll_line(&run, &manual, aed(1_000), aed(900)).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);

    let lines = run.lines(d.t.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].employee_id, manual.id);
    assert_eq!(
        (lines[0].fixed_wage, lines[0].net_amount),
        (aed(1_000), aed(900))
    );
}

/// Lines of employees with an open absence case are marked held; the hold
/// ends with the case.
#[renox::test]
async fn hrm16_an_open_absence_case_holds_the_line_until_it_is_closed() {
    let d = desk().await;
    let e = d.t.employee_in(78, EmployeeStatus::Active).await;
    d.t.salary(&e, WAGE, aed(15)).await;
    d.t.open_case(&e, support::nov(10), support::nov(1)).await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);
    let line = d.t.line_of(&run, &e).await;
    assert_eq!(line.status, LineStatus::Held);
    assert_eq!(line.held_reason.as_deref(), Some("absence case"));

    d.t.state()
        .emit(AbsenceCaseClosed {
            employee: e.employee_id(),
            case: Subject::new("ticketing.ticket", 1),
            closed_on: support::nov(12),
            reason: "returned".into(),
        })
        .await
        .unwrap();
    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);
    assert_eq!(d.t.line_of(&run, &e).await.status, LineStatus::Included);
}

/// The approver cannot calculate (`payroll.prepare`), and the preparer cannot
/// approve what they calculated (`HRM.ACL.04`).
#[renox::test]
async fn hrm16_calculate_is_for_the_preparer_and_approve_for_another_user() {
    let d = desk().await;
    let e = d.t.employee_in(79, EmployeeStatus::Active).await;
    d.t.salary(&e, WAGE, aed(15)).await;
    let run = d.t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    d.t.transition(&d.appr, &run, "calculate", "")
        .await
        .assert_status(403);
    d.t.transition(&d.prep, &run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.prep, &run, "approve", "")
        .await
        .assert_status(403);
    assert_eq!(d.t.run(&run).await.prepared_by, Some(d.prep.id));
}
