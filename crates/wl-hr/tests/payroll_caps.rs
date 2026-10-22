//! BL-HRM-15 (#205), `HRM.ACL.06`: a salary recovery stays within the Art. 25
//! caps per type and 50% in total per payroll line. A line above them is an
//! exception of the run, nothing is trimmed, and `approve` is refused for
//! every approver. Also the base it stands on: `calculate` by the preparer
//! and `approve` by another user (`HRM.ACL.04`).

mod support;

use renox::prelude::*;
use wl_hr::caps::{self, CapScope};
use wl_hr::{
    Adjustment, AdjustmentKind, Employee, EmployeeStatus, PayrollLine, PayrollRun, PayrollRunStatus,
};

use support::Booted;

/// Wage AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// AED `dirhams` in fils.
fn aed(dirhams: i64) -> i64 {
    dirhams * 100
}

/// A November run in `draft` with one line (wage AED 3,000) for employee E,
/// a preparer, an approver and a user who holds every payroll permission.
struct Desk {
    t: Booted,
    prep: User,
    appr: User,
    all: User,
    e: Employee,
    run: PayrollRun,
    line: PayrollLine,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    let all = t.user_with("Zed", "payroll_all").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;
    let line = t.payroll_line(&run, &e, WAGE, WAGE).await;
    Desk {
        t,
        prep,
        appr,
        all,
        e,
        run,
        line,
    }
}

/// The preparer calculates; the approver tries to approve.
async fn calculate_then_approve(d: &Desk) -> u16 {
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.appr, &d.run, "approve", "")
        .await
        .status
        .as_u16()
}

async fn adjustments(d: &Desk) -> Vec<Adjustment> {
    Adjustment::query()
        .where_(Adjustment::EMPLOYEE_ID.eq(d.e.id))
        .order_by(Adjustment::ID)
        .get(d.t.db())
        .await
        .unwrap()
}

/// AC1: wage AED 3,000 and an overpayment recovery of AED 700 this month: the
/// line is flagged (cap AED 600) and run `approve` is refused; nothing is
/// trimmed.
#[renox::test]
async fn hrm15_ac1_an_overpayment_recovery_above_the_cap_flags_the_line_and_refuses_approve() {
    let d = desk().await;
    d.t.deduct(
        &d.e,
        AdjustmentKind::OverpaymentRecovery,
        aed(700),
        "2026-11",
    )
    .await;

    assert_eq!(calculate_then_approve(&d).await, 422);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
    let problems = caps::problems(d.t.db(), d.run.id).await.unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line_id, d.line.id);
    assert_eq!(problems[0].scope, CapScope::Overpayment);
    assert_eq!((problems[0].amount, problems[0].cap), (aed(700), aed(600)));
    // Refused, not trimmed.
    assert_eq!(adjustments(&d).await[0].amount, aed(700));

    // The run page shows the exception, with Approve disabled.
    d.t.app.acting_as(&d.appr);
    d.t.app
        .get(&format!("/hr/payroll/{}", d.run.id))
        .await
        .assert_ok()
        .assert_see("Lines above the Art. 25 caps")
        .assert_see("Overpayment recovery (20%)")
        .assert_see("Reschedule its recoveries first");
}

/// AC1, the other side: the same recovery at the cap (AED 600) passes.
#[renox::test]
async fn hrm15_ac1_a_recovery_at_the_cap_is_approved() {
    let d = desk().await;
    d.t.deduct(
        &d.e,
        AdjustmentKind::OverpaymentRecovery,
        aed(600),
        "2026-11",
    )
    .await;

    assert_eq!(calculate_then_approve(&d).await, 303);

    let run = d.t.run(&d.run).await;
    assert_eq!(run.status, PayrollRunStatus::Approved);
    assert_eq!(run.approved_by, Some(d.appr.id));
    assert_eq!(run.prepared_by, Some(d.prep.id));
    d.t.app.acting_as(&d.prep);
    d.t.app
        .get(&format!("/hr/payroll/{}", d.run.id))
        .await
        .assert_ok()
        .assert_dont_see("Lines above the Art. 25 caps");
}

/// AC2: wage AED 3,000, overpayment recovery AED 600, court debt AED 750, fine
/// AED 150 and advance recovery AED 100 (each within its type cap): the total
/// AED 1,600 exceeds 50% (AED 1,500) and approval is refused.
#[renox::test]
async fn hrm15_ac2_deductions_each_within_their_cap_can_exceed_fifty_percent_in_total() {
    let d = desk().await;
    for (kind, dirhams) in [
        (AdjustmentKind::OverpaymentRecovery, 600),
        (AdjustmentKind::CourtDebt, 750),
        (AdjustmentKind::Fine, 150),
        (AdjustmentKind::AdvanceRecovery, 100),
    ] {
        d.t.deduct(&d.e, kind, aed(dirhams), "2026-11").await;
    }

    assert_eq!(calculate_then_approve(&d).await, 422);

    let problems = caps::problems(d.t.db(), d.run.id).await.unwrap();
    assert_eq!(problems.len(), 1, "only the total is exceeded");
    assert_eq!(problems[0].scope, CapScope::Total);
    assert_eq!(
        (problems[0].amount, problems[0].cap),
        (aed(1600), aed(1500))
    );
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
    // Without the advance recovery the total is AED 1,500 and it passes.
    let advance = adjustments(&d).await.pop().unwrap();
    Adjustment::query()
        .where_(Adjustment::ID.eq(advance.id))
        .update(d.t.db(), &[("amount", &aed(0))])
        .await
        .unwrap();
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.appr, &d.run, "approve", "")
        .await
        .assert_status(303);
}

/// AC3: wage AED 3,000 (a day's wage AED 100): a damage recovery of AED 600 is
/// flagged (cap AED 500); AED 500 passes.
#[renox::test]
async fn hrm15_ac3_damage_is_capped_at_five_days_wage() {
    let d = desk().await;
    let damage =
        d.t.deduct(&d.e, AdjustmentKind::DamageRecovery, aed(600), "2026-11")
            .await;

    assert_eq!(calculate_then_approve(&d).await, 422);
    let problems = caps::problems(d.t.db(), d.run.id).await.unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, CapScope::Damage);
    assert_eq!((problems[0].amount, problems[0].cap), (aed(600), aed(500)));

    Adjustment::query()
        .where_(Adjustment::ID.eq(damage.id))
        .update(d.t.db(), &[("amount", &aed(500))])
        .await
        .unwrap();
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);
    assert!(caps::problems(d.t.db(), d.run.id).await.unwrap().is_empty());
    d.t.transition(&d.appr, &d.run, "approve", "")
        .await
        .assert_status(303);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Approved);
}

/// AC4: an approver who holds every permission cannot approve a run with a
/// line above the caps, and the preparer cannot approve their own run.
#[renox::test]
async fn hrm15_ac4_no_permission_gets_past_the_caps_and_the_preparer_never_approves() {
    let d = desk().await;
    d.t.deduct(&d.e, AdjustmentKind::Fine, aed(200), "2026-11")
        .await;
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);

    // The user with every payroll permission, who did not prepare it.
    d.t.transition(&d.all, &d.run, "approve", "")
        .await
        .assert_status(422);
    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
    assert_eq!(d.t.run(&d.run).await.approved_by, None);

    // The same user as the preparer: refused for the segregation of duties.
    d.t.transition(&d.all, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.all, &d.run, "approve", "")
        .await
        .assert_status(422);
    assert_eq!(d.t.run(&d.run).await.prepared_by, Some(d.all.id));
}

/// A recovery of another month, or of another employee, is not counted on the
/// line.
#[renox::test]
async fn hrm15_only_the_recoveries_of_the_period_and_the_employee_count() {
    let d = desk().await;
    let other = d.t.employee_in(62, EmployeeStatus::Active).await;
    d.t.deduct(&d.e, AdjustmentKind::Fine, aed(200), "2026-12")
        .await;
    d.t.deduct(&other, AdjustmentKind::Fine, aed(200), "2026-11")
        .await;

    assert_eq!(calculate_then_approve(&d).await, 303);
}

/// Regression, `HRM.ACL.06`: an adjustment entered after `calculate` is taken
/// into the run at `approve`, so the caps see it and the approver cannot get
/// past them.
#[renox::test]
async fn hrm15_a_recovery_entered_after_calculate_still_meets_the_caps_at_approve() {
    let d = desk().await;
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);
    d.t.deduct(
        &d.e,
        AdjustmentKind::OverpaymentRecovery,
        aed(700),
        "2026-11",
    )
    .await;

    d.t.transition(&d.appr, &d.run, "approve", "")
        .await
        .assert_status(422);

    assert_eq!(d.t.run(&d.run).await.status, PayrollRunStatus::Calculated);
    assert_eq!(d.t.run(&d.run).await.approved_by, None);
}

/// Regression, the other side: a late recovery within the caps is linked to the
/// run and deducted from the net of the line at `approve`.
#[renox::test]
async fn hrm15_a_late_recovery_within_the_caps_is_deducted_at_approve() {
    let d = desk().await;
    d.t.transition(&d.prep, &d.run, "calculate", "")
        .await
        .assert_status(303);
    let late =
        d.t.deduct(&d.e, AdjustmentKind::Fine, aed(100), "2026-11")
            .await;

    d.t.transition(&d.appr, &d.run, "approve", "")
        .await
        .assert_status(303);

    let late = Adjustment::find_or_404(d.t.db(), late.id).await.unwrap();
    assert_eq!(late.run_id, Some(d.run.id));
    let line = d.t.line(&d.line).await;
    assert_eq!(line.deductions_amount, aed(100));
    assert_eq!(line.net_amount, WAGE - aed(100));
}
