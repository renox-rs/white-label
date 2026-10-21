//! BL-HRM-16 (#206), difference lines: approved adjustment timesheets that
//! affect payroll and the reconciliation of provisional hours enter the next
//! regular run of the employee's entity (wl-hr §5.3 step 2): + hours are
//! paid, - hours are recovered within the Art. 25 caps (`HRM.ACL.06`), both at
//! the rates of the day of the hours, each used once.

mod support;

use renox::prelude::*;
use wl_hr::caps::{self, CapScope};
use wl_hr::{
    Adjustment, AdjustmentKind, Employee, EmployeeStatus, HourDifference, PayrollRun,
    PayrollRunStatus,
};
use wl_kernel::Subject;
use wl_kernel::attendance::{CategoryMinutes, TimesheetApproved};
use wl_kernel::timesheets::TimesheetReconciliationNeeded;

use support::{Booted, nov};

/// A wage of AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// AED `dirhams` in fils.
fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

/// Employee E on AED 3,000 and AED 15 an hour, the paid November run, a
/// December run in `draft`, a preparer and an approver.
struct Desk {
    t: Booted,
    prep: User,
    appr: User,
    e: Employee,
    november: PayrollRun,
    december: PayrollRun,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(81, EmployeeStatus::Active).await;
    t.salary(&e, WAGE, aed(15)).await;
    let november = t.payroll_run("2026-11", PayrollRunStatus::Paid).await;
    let december = t.payroll_run("2026-12", PayrollRunStatus::Draft).await;
    Desk {
        t,
        prep,
        appr,
        e,
        november,
        december,
    }
}

fn minutes(category: &str, minutes: i64) -> CategoryMinutes {
    CategoryMinutes {
        category: category.into(),
        minutes,
    }
}

impl Desk {
    /// Timesheet 3, paid on provisional hours by the November run, was
    /// approved and differs by `delta` minutes of normal hours; the
    /// adjustment timesheet is `adjustment`.
    async fn reconcile(&self, adjustment: i64, delta: i64) {
        self.t
            .state()
            .emit(TimesheetReconciliationNeeded {
                timesheet: Subject::new("attendance.timesheet", 3),
                owner: self.e.employee_id(),
                payroll_run: self.november.subject(),
                provisional: vec![minutes("normal", 9_600)],
                approved: vec![minutes("normal", 9_600 + delta)],
                delta: vec![minutes("normal", delta)],
                adjustment: Subject::new("attendance.timesheet", adjustment),
            })
            .await
            .unwrap();
    }

    /// An adjustment timesheet `timesheet` was approved with `totals`, for
    /// `affects`.
    async fn approve_adjustment(
        &self,
        timesheet: i64,
        totals: Vec<CategoryMinutes>,
        affects: &str,
    ) {
        self.t
            .state()
            .emit(TimesheetApproved {
                timesheet: Subject::new("attendance.timesheet", timesheet),
                owner: self.e.employee_id(),
                period_from: nov(1),
                period_to: nov(30),
                context: None,
                totals,
                approver_kind: "internal".into(),
                approval_basis: "approved".into(),
                kind: "adjustment".into(),
                adjusts: Some(Subject::new("attendance.timesheet", 3)),
                origin: Some("payroll_error".into()),
                affects: Some(affects.into()),
            })
            .await
            .unwrap();
    }

    async fn calculate(&self, run: &PayrollRun) {
        self.t
            .transition(&self.prep, run, "calculate", "")
            .await
            .assert_status(303);
    }

    async fn adjustments(&self) -> Vec<Adjustment> {
        Adjustment::query()
            .where_(Adjustment::EMPLOYEE_ID.eq(self.e.id))
            .order_by(Adjustment::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// AC3: wage AED 3,000 and a -50 h reconciliation at AED 15/h (-AED 750, above
/// the AED 600 overpayment cap): the line is flagged and `approve` refused;
/// after the preparer converts it to a recovery of AED 600 now and AED 150
/// next month, `approve` succeeds.
#[renox::test]
async fn hrm16_ac3_a_negative_difference_above_the_cap_is_flagged_until_it_is_made_a_plan() {
    let d = desk().await;
    d.reconcile(9, -3_000).await;

    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!(line.difference_amount, -aed(750));
    assert_eq!(line.deductions_amount, aed(750));
    let problems = caps::problems(d.t.db(), d.december.id).await.unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, CapScope::Overpayment);
    assert_eq!((problems[0].amount, problems[0].cap), (aed(750), aed(600)));
    // Created in full, not trimmed, and `approve` is refused.
    assert_eq!(d.adjustments().await[0].amount, aed(750));
    d.t.transition(&d.appr, &d.december, "approve", "")
        .await
        .assert_status(422);

    let difference = HourDifference::of_run(d.t.db(), d.december.id)
        .await
        .unwrap()
        .remove(0);
    d.t.app.acting_as(&d.prep);
    d.t.app
        .post(
            &format!("/hr/payroll/differences/{}/convert", difference.id),
            &[("note", "")],
        )
        .await
        .assert_status(303);

    let plan = d.adjustments().await;
    let rows: Vec<(String, i64)> = plan.iter().map(|a| (a.period.clone(), a.amount)).collect();
    assert_eq!(
        rows,
        vec![
            ("2026-12".to_owned(), aed(600)),
            ("2027-01".to_owned(), aed(150))
        ]
    );
    assert!(
        plan.iter()
            .all(|a| a.kind == AdjustmentKind::OverpaymentRecovery)
    );

    d.calculate(&d.december).await;
    d.t.transition(&d.appr, &d.december, "approve", "")
        .await
        .assert_status(303);
    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!(line.deductions_amount, aed(600));
    assert_eq!(line.net_amount, WAGE - aed(600));
    // The second instalment waits for the January run.
    let plan = d.adjustments().await;
    assert_eq!(plan[0].run_id, Some(d.december.id));
    assert_eq!(plan[1].run_id, None);
}

/// AC4: a -8 h reconciliation at AED 15/h: December's line has -AED 120
/// within the caps.
#[renox::test]
async fn hrm16_ac4_a_small_negative_difference_is_recovered_within_the_caps() {
    let d = desk().await;
    d.reconcile(9, -480).await;

    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!(line.difference_amount, -aed(120));
    assert_eq!(line.deductions_amount, aed(120));
    assert_eq!(line.net_amount, WAGE - aed(120));
    assert!(
        caps::problems(d.t.db(), d.december.id)
            .await
            .unwrap()
            .is_empty()
    );
    d.t.transition(&d.appr, &d.december, "approve", "")
        .await
        .assert_status(303);
}

/// AC5: an approved `payroll` adjustment of +4 h overtime at 125% of AED
/// 15/h: December pays AED 75; the adjustment timesheet is told as used when
/// the file is sent.
#[renox::test]
async fn hrm16_ac5_an_approved_payroll_adjustment_is_paid_at_the_overtime_rate() {
    let d = desk().await;
    d.approve_adjustment(9, vec![minutes("overtime", 240)], "payroll")
        .await;

    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!(line.difference_amount, aed(75));
    assert_eq!(line.net_amount, WAGE + aed(75));
    d.t.transition(&d.appr, &d.december, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(d.t.state(), d.december.id)
        .await
        .unwrap();

    let used = d.t.timesheets.used();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].0, d.december.subject());
    assert_eq!(used[0].1.len(), 1);
    assert_eq!(
        used[0].1[0].timesheet,
        Subject::new("attendance.timesheet", 9)
    );
    assert!(!used[0].1[0].provisional);
    assert_eq!(used[0].1[0].total_minutes(), 240);
}

/// The hours are priced with the structure effective on their day, not
/// today's: a rise from 1 December does not change November's hours.
#[renox::test]
async fn hrm16_difference_hours_are_priced_with_the_structure_of_the_day_of_the_hours() {
    let d = desk().await;
    wl_hr::SalaryStructure::create(
        d.t.db(),
        wl_hr::SalaryStructure {
            employee_id: d.e.id,
            effective_from: chrono_day(2026, 12, 1),
            basic: WAGE,
            hourly_rate: Some(aed(20)),
            ..wl_hr::SalaryStructure::default()
        },
    )
    .await
    .unwrap();
    d.approve_adjustment(9, vec![minutes("overtime", 240)], "payroll")
        .await;

    d.calculate(&d.december).await;

    assert_eq!(
        d.t.line_of(&d.december, &d.e).await.difference_amount,
        aed(75)
    );
}

fn chrono_day(y: i32, m: u32, day: u32) -> renox::chrono::NaiveDate {
    renox::chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// AC6: recalculating December twice does not double the line; the
/// difference is linked to the run and a later run does not take it again.
#[renox::test]
async fn hrm16_ac6_recalculating_does_not_double_the_line_and_a_delta_is_used_once() {
    let d = desk().await;
    d.reconcile(9, -480).await;
    d.approve_adjustment(10, vec![minutes("overtime", 240)], "payroll")
        .await;

    d.calculate(&d.december).await;
    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!(line.difference_amount, aed(75) - aed(120));
    assert_eq!(line.deductions_amount, aed(120));
    assert_eq!(d.adjustments().await.len(), 1);
    let linked = HourDifference::of_run(d.t.db(), d.december.id)
        .await
        .unwrap();
    assert_eq!(linked.len(), 2);

    // The January run finds nothing left to pay or recover.
    let january = d.t.payroll_run("2027-01", PayrollRunStatus::Draft).await;
    d.calculate(&january).await;
    let line = d.t.line_of(&january, &d.e).await;
    assert_eq!((line.difference_amount, line.deductions_amount), (0, 0));
    assert_eq!(d.adjustments().await.len(), 1);
    assert_eq!(
        HourDifference::of_run(d.t.db(), d.december.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

/// The same event twice keeps one difference.
#[renox::test]
async fn hrm16_the_same_event_twice_keeps_one_difference() {
    let d = desk().await;
    d.reconcile(9, -480).await;
    d.reconcile(9, -480).await;
    d.approve_adjustment(10, vec![minutes("overtime", 240)], "both")
        .await;
    d.approve_adjustment(10, vec![minutes("overtime", 240)], "both")
        .await;

    d.calculate(&d.december).await;

    let linked = HourDifference::of_run(d.t.db(), d.december.id)
        .await
        .unwrap();
    assert_eq!(linked.len(), 2);
}

/// AC7: a `billing`-only adjustment is ignored by payroll.
#[renox::test]
async fn hrm16_ac7_a_billing_only_adjustment_is_ignored_by_payroll() {
    let d = desk().await;
    d.approve_adjustment(9, vec![minutes("overtime", 240)], "billing")
        .await;

    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.december, &d.e).await;
    assert_eq!((line.difference_amount, line.net_amount), (0, WAGE));
    assert!(
        HourDifference::of_run(d.t.db(), d.december.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A paid run is not edited: a difference found after it is paid goes to the
/// next regular run, and the paid run keeps its lines.
#[renox::test]
async fn hrm16_a_paid_run_keeps_its_lines_when_a_difference_arrives() {
    let d = desk().await;
    d.t.payroll_line(&d.november, &d.e, WAGE, WAGE).await;
    d.reconcile(9, -480).await;

    d.calculate(&d.december).await;

    let line = d.t.line_of(&d.november, &d.e).await;
    assert_eq!((line.net_amount, line.deductions_amount), (WAGE, 0));
    assert_eq!(d.t.run(&d.november).await.status, PayrollRunStatus::Paid);
}
