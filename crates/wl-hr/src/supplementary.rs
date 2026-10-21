//! Correcting a paid run with a supplementary run (wl-hr §3 *Payroll run*:
//! type, corrects; §4 *Run types*; §5.6 step 1; `HRM.ACL.04`, `HRM.ACL.06`;
//! issue #207).
//!
//! An error found after `paid` is corrected by a run of type `supplementary`
//! whose `corrects` is the **original** paid run, which is never edited
//! ([`create`]). It holds only the affected employees: one line each, with
//! the difference as its net (+ underpaid, - overpaid). A paid run may get
//! several; each difference is against the paid run **plus the earlier
//! corrections**, and a correction of a supplementary run names the original.
//!
//! The run goes through the machine as any run: `calculate` records the
//! preparer (the lines are fixed, nothing is recalculated), `approve` by
//! another user (`HRM.ACL.04`), `submit` sends only the positive lines to the
//! bank, in the run's own SIF (none when every difference is negative, which
//! then needs no bank confirmation at `confirm_paid`).
//!
//! At `confirm_paid`, when the correction is final ([`settle_negatives_in`],
//! called by [`crate::wps_partial`]):
//!
//! - every line is `paid` and its corrected payslip is published (a line a
//!   `submit` left out because it was held waits for `HRM.RUL.12`);
//! - a **negative difference** becomes an *overpayment recovery* spread over
//!   the next regular runs, at most 20% of the wage a month and within the 50%
//!   total ([`crate::recovery`], `HRM.ACL.06`); for an employee who has left
//!   since it is added to the final settlement's recoveries while the
//!   settlement is not `paid`, else it is a receivable on the employee's party
//!   ([`crate::receivable`]);
//! - the cost difference is posted to the cost objects of the paid run's
//!   lines ([`crate::payroll_events`]).
//!
//! Decisions:
//!
//! - **Input.** The preparer gives the corrected net of each affected
//!   employee and the component it is in; the difference is computed against
//!   what was paid. An employee whose difference is zero is not affected and
//!   gets no line.
//! - **First instalment.** The month after the latest regular run of the legal
//!   entity that can no longer take an adjustment (approved or beyond), and not
//!   before the month after the corrected period.
//! - **Cost objects and wage.** A line copies the cost object and the fixed
//!   wage of the employee's line in the paid run; the fixed wage is the basis
//!   of the caps of the recovery.

use renox::db::{Transaction, now};
use renox::prelude::*;

use crate::employee::{Employee, EmployeeStatus};
use crate::hours::next_period;
use crate::payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use crate::receivable::Receivable;
use crate::recovery;
use crate::salary::SalaryStructure;
use crate::settlement::{FinalSettlement, SettlementStatus};
use crate::texts::english;
use crate::{calculation, permissions};

/// The run type of a correction of a paid run.
pub const SUPPLEMENTARY: &str = "supplementary";

fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// What the preparer says about one affected employee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// The employee.
    pub employee_id: i64,
    /// What the employee should have been paid in the run, AED fils.
    pub correct_net: i64,
    /// The component the difference is in (overtime, allowance, ...).
    pub component: String,
}

/// What was paid to `employee_id` for the period of the original run `original`:
/// the nets of their `paid` lines in it and in the earlier supplementary runs.
async fn paid_so_far(tx: &mut Transaction, original: i64, employee_id: i64) -> Result<i64> {
    let mut runs = vec![original];
    runs.extend(
        PayrollRun::query()
            .where_(PayrollRun::CORRECTS.eq(original))
            .get(&mut *tx)
            .await?
            .into_iter()
            .map(|r| r.id),
    );
    Ok(PayrollLine::query()
        .where_(PayrollLine::RUN_ID.is_in(runs))
        .where_(PayrollLine::EMPLOYEE_ID.eq(employee_id))
        .get(&mut *tx)
        .await?
        .iter()
        .filter(|l| l.status == LineStatus::Paid || l.run_id != original)
        .map(|l| l.net_amount)
        .sum())
}

/// Makes a draft supplementary run correcting the paid run `run_id`, with a
/// line per affected employee. `user` needs `payroll.prepare`.
///
/// # Errors
///
/// 422 when the run is not `paid`, the reason is blank, or no employee has a
/// difference.
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    run_id: i64,
    reason: &str,
    corrections: &[Correction],
) -> Result<PayrollRun> {
    if !user.has_permission(permissions::PAYROLL_PREPARE) {
        return Err(Error::Forbidden);
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refuse("wl_hr.payroll.supplementary_reason_required"));
    }
    let named = PayrollRun::find_or_404(&state.db, run_id).await?;
    if named.status != PayrollRunStatus::Paid {
        return Err(refuse("wl_hr.payroll.supplementary_run_not_paid"));
    }
    // `corrects` always names the original run.
    let original_id = named.corrects.unwrap_or(named.id);
    let original = PayrollRun::find_or_404(&state.db, original_id).await?;
    let mut tx = state.db.begin().await?;
    let mut lines = Vec::new();
    for correction in corrections {
        if correction.correct_net < 0
            || lines
                .iter()
                .any(|l: &PayrollLine| l.employee_id == correction.employee_id)
        {
            return Err(refuse("wl_hr.payroll.supplementary_correction_invalid"));
        }
        let paid = paid_so_far(&mut tx, original.id, correction.employee_id).await?;
        let difference = correction.correct_net - paid;
        if difference == 0 {
            continue;
        }
        let of_original = PayrollLine::query()
            .where_(PayrollLine::RUN_ID.eq(original.id))
            .where_(PayrollLine::EMPLOYEE_ID.eq(correction.employee_id))
            .order_by(PayrollLine::ID)
            .first(&mut tx)
            .await?;
        lines.push(PayrollLine {
            employee_id: correction.employee_id,
            net_amount: difference,
            fixed_wage: of_original.as_ref().map_or(0, |l| l.fixed_wage),
            component: Some(correction.component.trim().to_owned()).filter(|c| !c.is_empty()),
            cost_object_type: of_original
                .as_ref()
                .and_then(|l| l.cost_object_type.clone()),
            cost_object_id: of_original.as_ref().and_then(|l| l.cost_object_id),
            status: LineStatus::Included,
            ..PayrollLine::default()
        });
    }
    if lines.is_empty() {
        return Err(refuse("wl_hr.payroll.supplementary_nothing_to_correct"));
    }
    let mut run = PayrollRun {
        legal_entity_id: original.legal_entity_id,
        period: original.period.clone(),
        run_type: SUPPLEMENTARY.to_owned(),
        corrects: Some(original.id),
        reason: Some(reason.to_owned()),
        ..PayrollRun::default()
    };
    run.insert(&mut tx).await?;
    for mut line in lines {
        line.run_id = run.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(run)
}

/// The supplementary runs that correct `original`, oldest first.
pub async fn of_original(db: &Db, original: i64) -> Result<Vec<PayrollRun>> {
    PayrollRun::query()
        .where_(PayrollRun::CORRECTS.eq(original))
        .order_by(PayrollRun::ID)
        .get(db)
        .await
}

/// The period of the first instalment of a recovery caused by `run`.
async fn first_period_in(tx: &mut Transaction, run: &PayrollRun) -> Result<Option<String>> {
    let mut query = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq(calculation::REGULAR))
        .where_(PayrollRun::STATUS.is_in([
            PayrollRunStatus::Approved,
            PayrollRunStatus::Submitted,
            PayrollRunStatus::Rejected,
            PayrollRunStatus::PartiallyRejected,
            PayrollRunStatus::Paid,
        ]));
    query = match run.legal_entity_id {
        Some(entity) => query.where_(PayrollRun::LEGAL_ENTITY_ID.eq(entity)),
        None => query.where_(PayrollRun::LEGAL_ENTITY_ID.is_null()),
    };
    let base = match query
        .order_by_desc(PayrollRun::PERIOD)
        .first(&mut *tx)
        .await?
    {
        Some(latest) if latest.period > run.period => latest.period,
        _ => run.period.clone(),
    };
    Ok(next_period(&base))
}

/// Settles the negative differences of the supplementary run `run` once it is
/// paid, inside `tx`: a recovery plan, a settlement recovery or a receivable
/// for each. Does nothing for another type of run.
pub(crate) async fn settle_negatives_in(tx: &mut Transaction, run: &PayrollRun) -> Result {
    if run.run_type != SUPPLEMENTARY {
        return Ok(());
    }
    let lines = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run.id))
        .where_(PayrollLine::NET_AMOUNT.lt(0_i64))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?;
    let note = english(
        "wl_hr.payroll.supplementary_recovery_note",
        &[("run", &run.id)],
    );
    for line in lines {
        let amount = -line.net_amount;
        let Some(employee) = Employee::find(&mut *tx, line.employee_id).await? else {
            continue;
        };
        let settlements = FinalSettlement::query()
            .where_(FinalSettlement::EMPLOYEE_ID.eq(employee.id))
            .where_(FinalSettlement::STATUS.ne(SettlementStatus::Cancelled))
            .order_by_desc(FinalSettlement::ID)
            .get(&mut *tx)
            .await?;
        let open = settlements.iter().find(|s| {
            matches!(
                s.status,
                SettlementStatus::Draft | SettlementStatus::Approved | SettlementStatus::Disputed
            )
        });
        if let Some(open) = open {
            FinalSettlement::query()
                .where_(FinalSettlement::ID.eq(open.id))
                .update(
                    &mut *tx,
                    &[
                        ("recoveries", &(open.recoveries + amount)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            continue;
        }
        let left = !settlements.is_empty()
            || matches!(
                employee.status,
                EmployeeStatus::Exited
                    | EmployeeStatus::Deceased
                    | EmployeeStatus::OnboardingCancelled
            );
        let mut owed = amount;
        if !left {
            let wage = match line.fixed_wage {
                0 => SalaryStructure::effective_in(
                    &mut *tx,
                    employee.id,
                    wl_kernel::documents::today(),
                )
                .await?
                .map_or(0, |s| s.fixed_wage()),
                wage => wage,
            };
            if let Some(first) = first_period_in(&mut *tx, run).await? {
                let (_, left_over) = recovery::create_plan_in(
                    &mut *tx,
                    employee.id,
                    wage,
                    amount,
                    &first,
                    None,
                    &note,
                )
                .await?;
                owed = left_over;
            }
        }
        if owed > 0 {
            let mut receivable = Receivable {
                employee_id: employee.id,
                party_id: employee.party_id,
                amount: owed,
                source_run_id: Some(run.id),
                source_line_id: Some(line.id),
                status: "open".to_owned(),
                note: Some(note.clone()),
                ..Receivable::default()
            };
            receivable.insert(&mut *tx).await?;
        }
    }
    Ok(())
}
