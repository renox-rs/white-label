//! Payroll runs and lines (wl-hr §4 *Payroll run*, §5.3, §5.6, issues #197,
//! #201 to #208). The run and its lines, and:
//!
//! - `calculate` (draft, calculated -> calculated, `payroll.prepare`): the
//!   user is the preparer; the lines of a `regular` run are calculated from
//!   the salary structures, the timesheet hours (provisional ones marked) and
//!   the differences of adjustment timesheets and reconciliations
//!   ([`crate::calculation`], BL-HRM-16); the period's adjustments are taken
//!   into the lines.
//! - `approve` (calculated -> approved, `payroll.approve`): another user than
//!   the preparer (`HRM.ACL.04`), and refused while a line is above the
//!   Art. 25 caps (`HRM.ACL.06`, [`crate::caps`]) for every approver.
//! - `return` (calculated, approved -> draft, `payroll.approve`): the approver
//!   sends the run back with a reason and a comment on at least one line
//!   ([`comment_line`]); the approver is cleared and the preparer told
//!   ([`crate::payroll_events`]). Not possible once the file was sent.
//!
//! - `confirm_paid` (submitted, partially_rejected -> paid, `payroll.approve`)
//!   and the partly failed file: see [`crate::wps_partial`].
//! - `record_rejection` (submitted -> rejected) and `resubmit` (rejected ->
//!   submitted), `payroll.prepare`: the bank rejects the whole file and the
//!   preparer sends a new version for the same lines ([`crate::wps_rejection`]);
//!   the versions are [`crate::sif::SifFile`]s and are never overwritten.
//!
//! The transitions that decide which line goes into which SIF file:
//!
//! - `submit` (approved -> submitted): generates SIF version 1 from the lines
//!   that are `included`. A `held` line is left out, so no line is put in a
//!   new SIF file while its hold stands (an absence report, MR 340/2026
//!   Art. 4).
//! - `record_partial` (submitted -> partially_rejected): the bank's result.
//!   The lines it shows failed are `failed` with their reason, but the line of
//!   an employee who is `absconded` is `held` with *absence report* instead;
//!   the other lines of the file are `paid`.
//! - `regenerate` (partially_rejected -> submitted): SIF version n+1 with
//!   only the lines that were fixed and are `included` again
//!   ([`fix_line`]); a `failed` line left refuses it, a `held` line stays out.
//!
//! `submit` is a `system` transition run by [`submit`] (the user-facing one
//! comes with the submit guard of the payroll base). `record_partial` and
//! `regenerate` are user transitions of `payroll.prepare` since BL-HRM-13
//! ([`crate::wps_partial`], which also holds `confirm_paid`); the functions
//! [`record_partial`] and [`regenerate`] run their system twins
//! ([`RECORD_PARTIAL_BY_SYSTEM`], [`REGENERATE_BY_SYSTEM`]) with the same
//! effects.
//!
//! [`hold_open_lines_in`] is what the exits use: the lines that are
//! `included` in runs not yet `submitted` (`draft`, `calculated`, `approved`)
//! become `held` with a reason; a line in a file already sent stays.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};

use crate::employee::{Employee, EmployeeStatus};
use crate::texts::english;
use crate::{adjustment, caps, permissions, sif, wps_partial, wps_rejection};

/// The reason of a line held by the absence report.
pub const HELD_ABSENCE_REPORT: &str = "absence report";

/// The reason of a line held while the employee has an open absence case
/// (wl-hr §4 *Payroll run* `calculate`).
pub const HELD_ABSENCE_CASE: &str = "absence case";

/// The preparer calculates the lines of the run.
pub const CALCULATE: &str = "calculate";
/// A second user approves the calculated run.
pub const APPROVE: &str = "approve";
/// The approver sends a calculated or approved run back to draft.
pub const RETURN: &str = "return";
/// The bank rejected the whole file.
pub const RECORD_REJECTION: &str = "record_rejection";
/// A new version of the rejected file, once the cause is fixed.
pub const RESUBMIT: &str = "resubmit";
/// The run is ready to be sent.
pub const SUBMIT: &str = "submit";
/// The bank's result file shows failed lines (`payroll.prepare`).
pub const RECORD_PARTIAL: &str = "record_partial";
/// The same as [`RECORD_PARTIAL`], run by the system (a bank result that
/// arrives without a user, and the tests of the exits): it needs no result
/// file.
pub const RECORD_PARTIAL_BY_SYSTEM: &str = "record_partial_by_system";
/// A new SIF file with the fixed lines (`payroll.prepare`).
pub const REGENERATE: &str = "regenerate";
/// The same as [`REGENERATE`], run by the system.
pub const REGENERATE_BY_SYSTEM: &str = "regenerate_by_system";
/// Payment is confirmed for every line not held (`payroll.approve`).
pub const CONFIRM_PAID: &str = "confirm_paid";

/// Where a payroll run stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PayrollRunStatus {
    /// Being prepared.
    #[default]
    Draft,
    /// Lines calculated.
    Calculated,
    /// Approved by a second user.
    Approved,
    /// The SIF file was sent.
    Submitted,
    /// The bank or exchange house rejected the whole file.
    Rejected,
    /// The bank paid some lines and failed others.
    PartiallyRejected,
    /// Payment confirmed.
    Paid,
}

impl PayrollRunStatus {
    /// Whether the run has not been sent yet: its lines can still be held.
    pub fn is_unsent(self) -> bool {
        matches!(
            self,
            PayrollRunStatus::Draft | PayrollRunStatus::Calculated | PayrollRunStatus::Approved
        )
    }
}

/// One pay period for one legal entity.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_payroll_runs", state_machine, index(period, status))]
pub struct PayrollRun {
    /// The primary key.
    pub id: i64,
    /// The employing legal entity.
    pub legal_entity_id: Option<i64>,
    /// The month, e.g. `2026-11`.
    pub period: String,
    /// `regular`, `off_cycle` or `supplementary`.
    pub run_type: String,
    /// Where it stands; changed only by the machine.
    pub status: PayrollRunStatus,
    /// How many SIF versions were generated.
    pub sif_version: i64,
    /// The user who last ran `calculate`: the preparer (`HRM.ACL.04`).
    pub prepared_by: Option<i64>,
    /// The user who approved it; empty after `return`.
    pub approved_by: Option<i64>,
    /// Why the approver last sent it back to `draft` (`return`).
    pub return_reason: Option<String>,
    /// Why the bank rejected the whole file (`file_format`,
    /// `establishment_id`, `funding`, `other`).
    pub rejection_reason: Option<String>,
    /// The bank's rejection notice (a reference to the attached file).
    pub rejection_notice: Option<String>,
    /// The note on the fix entered at `resubmit`.
    pub resubmit_note: Option<String>,
    /// The day the MD and the accountant were told that the rejection is
    /// still open before the WPS due date (`HRM.RUL.11`, once per run).
    pub escalated_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Where a payroll line stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineStatus {
    /// Goes into the next SIF file.
    #[default]
    Included,
    /// Left out, with a reason.
    Held,
    /// The bank paid it.
    Paid,
    /// The bank failed it.
    Failed,
}

/// The pay of one employee in a run.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_payroll_lines",
    index(run_id, status),
    index(employee_id)
)]
pub struct PayrollLine {
    /// The primary key.
    pub id: i64,
    /// The run.
    #[model(references = "wl_hr_payroll_runs")]
    pub run_id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The net pay, AED fils.
    pub net_amount: i64,
    /// The gross fixed wage for the period (basic plus fixed allowances), AED
    /// fils: the basis of the Art. 25 caps (`HRM.ACL.06`). The payroll
    /// calculation fills it from the salary structure.
    pub fixed_wage: i64,
    /// The pay of the hours that are not `normal` (overtime, night, rest day,
    /// public holiday), AED fils (BL-HRM-16).
    pub variable_amount: i64,
    /// The hours added (+) or taken back (-) by adjustment timesheets and
    /// reconciliations in this run, AED fils; the recoveries are among the
    /// deductions.
    pub difference_amount: i64,
    /// The adjustments deducted on this line, AED fils.
    pub deductions_amount: i64,
    /// The minutes of timesheet hours the line was calculated from.
    pub hours_minutes: i64,
    /// Of them, the minutes of timesheets not approved yet (provisional).
    pub provisional_minutes: i64,
    /// An exception the preparer reviews, e.g. `missing_timesheet`
    /// ([`crate::calculation::MISSING_TIMESHEET`]).
    pub exception: Option<String>,
    /// The approver's comment on this line to fix (`return`); cleared by the
    /// next `calculate`.
    pub approver_comment: Option<String>,
    /// Where it stands.
    pub status: LineStatus,
    /// Why it is held.
    pub held_reason: Option<String>,
    /// Why the bank failed it.
    pub failure_reason: Option<String>,
    /// The SIF version that carries it; empty when it is in none.
    pub sif_version: Option<i64>,
    /// When its payslip was published to the employee portal (once).
    pub payslip_published_at: Option<DateTime>,
    /// The approved non-WPS way a failed line was paid instead
    /// (`transfer`, `cheque`, `cash`).
    pub paid_method: Option<String>,
    /// That payment's reference, e.g. `TR-88`.
    pub paid_reference: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayrollRun {
    /// The run as a [`Subject`](wl_kernel::Subject), for notices.
    pub fn subject(&self) -> wl_kernel::Subject {
        wl_kernel::PayrollRunId(self.id).subject()
    }

    /// The lines of the run, oldest first.
    pub async fn lines(&self, db: &Db) -> Result<Vec<PayrollLine>> {
        PayrollLine::query()
            .where_(PayrollLine::RUN_ID.eq(self.id))
            .order_by(PayrollLine::ID)
            .get(db)
            .await
    }
}

/// A line the bank failed, with the reason from its result file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FailedLine {
    /// The line.
    pub line_id: i64,
    /// Why (invalid IBAN, account closed, ...).
    pub reason: String,
}

/// The input of `record_partial`.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct PartialInput {
    /// The lines the result file shows failed.
    pub failed: Vec<FailedLine>,
    /// The bank's result file: a reference to the attached file. It is the
    /// confirmation for the lines it shows paid. The user transition needs
    /// it; the system one does not.
    pub result_file: String,
}

/// The input of `confirm_paid`.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ConfirmInput {
    /// The bank or exchange-house confirmation of the latest SIF version: a
    /// reference to the attached file. Earlier versions were confirmed by
    /// the result file of `record_partial`.
    pub confirmation: String,
}

impl StateMachine for PayrollRun {
    type State = PayrollRunStatus;
    const COLUMN: renox::db::Col<PayrollRun, PayrollRunStatus> = PayrollRun::STATUS;

    fn machine() -> Machine<Self> {
        use PayrollRunStatus::{
            Approved, Calculated, Draft, Paid, PartiallyRejected, Rejected, Submitted,
        };
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Calculated)
            .tone("info")
            .state(Approved)
            .tone("info")
            .state(Submitted)
            .tone("info")
            .state(Rejected)
            .tone("error")
            .state(PartiallyRejected)
            .tone("warning")
            .state(Paid)
            .tone("success")
            .terminal()
            .transition(calculate())
            .transition(approve())
            .transition(return_run())
            .transition(wps_rejection::record_rejection())
            .transition(wps_rejection::resubmit())
            .transition(
                Transition::new(SUBMIT)
                    .label("Submit")
                    .system()
                    .from([Approved])
                    .to(Submitted)
                    .effect(|run: &PayrollRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move { generate_sif_in(tx, id).await })
                    }),
            )
            .transition(wps_partial::record_partial(false))
            .transition(wps_partial::record_partial(true))
            .transition(wps_partial::regenerate(false))
            .transition(wps_partial::regenerate(true))
            .transition(wps_partial::confirm_paid())
    }
}

/// `calculate` (draft, calculated -> calculated, `payroll.prepare`): the user
/// who runs it is the preparer. The lines of a `regular` run are calculated
/// from the salary structures, the timesheet hours the transition is given as
/// input and the differences of adjustment timesheets and reconciliations
/// ([`crate::calculation`]); the adjustments of the run's period are taken
/// into the lines (`HRM.ACL.06` checks them). The lines of an `off_cycle` or a
/// `supplementary` run are fixed when the run is made: `calculate` only
/// records the preparer.
fn calculate() -> Transition<PayrollRun> {
    Transition::new(CALCULATE)
        .label("Calculate")
        .permission(permissions::PAYROLL_PREPARE)
        .from([PayrollRunStatus::Draft, PayrollRunStatus::Calculated])
        .to(PayrollRunStatus::Calculated)
        .effect(|run: &PayrollRun, ctx, tx| {
            let run = run.clone();
            let by = ctx.actor().map(|user| user.id);
            let input = ctx
                .input_as::<crate::calculation::CalculateInput>()
                .unwrap_or_default();
            Box::pin(async move {
                let id = run.id;
                let calculated = crate::calculation::lines_in(tx, &run, &input).await?;
                if run.run_type == crate::calculation::REGULAR {
                    adjustment::link_to_run_in(tx, id, &run.period).await?;
                }
                crate::calculation::settle_in(tx, id, &calculated).await?;
                // The comments of a `return` were answered by this calculation.
                PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approver_comment", &None::<String>),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                PayrollRun::query()
                    .where_(PayrollRun::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("prepared_by", &by),
                            ("approved_by", &None::<i64>),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `approve` (calculated -> approved, `payroll.approve`): not the preparer
/// (`HRM.ACL.04`), and not while a line is above the Art. 25 caps
/// (`HRM.ACL.06`, [`caps`]): the check is an effect, so it cannot be skipped
/// by any permission.
fn approve() -> Transition<PayrollRun> {
    Transition::new(APPROVE)
        .label("Approve")
        .tone("primary")
        .permission(permissions::PAYROLL_APPROVE)
        .from([PayrollRunStatus::Calculated])
        .to(PayrollRunStatus::Approved)
        .guard("not_the_preparer", |run: &PayrollRun, ctx| {
            let problem = preparer_problem(run, ctx);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let problems = caps::problems_in(tx, id).await?;
                if !problems.is_empty() {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        english("wl_hr.payroll.caps_exceeded", &[("count", &problems.len())]),
                    ));
                }
                PayrollRun::query()
                    .where_(PayrollRun::ID.eq(id))
                    .update(&mut *tx, &[("approved_by", &by), ("updated_at", &now())])
                    .await?;
                Ok(())
            })
        })
}

/// Why the user may not approve or return `run`: they prepared it
/// (`HRM.ACL.04`).
fn preparer_problem(run: &PayrollRun, ctx: &TransitionCtx) -> Option<String> {
    let same = matches!(
        (ctx.actor(), run.prepared_by),
        (Some(user), Some(preparer)) if user.id == preparer
    );
    same.then(|| english("wl_hr.payroll.same_user", &[]))
}

/// `return` (calculated, approved -> draft, `payroll.approve`): the approver
/// sends the run back with a reason and at least one comment on a line to
/// fix. The approver is cleared; the lines and files stay, and the next
/// `calculate` recalculates the lines. Not possible once the file was sent:
/// an amount error is then corrected by a supplementary run (§5.6). The
/// preparer is told after the commit (`payroll_events`).
fn return_run() -> Transition<PayrollRun> {
    Transition::new(RETURN)
        .label("Return")
        .tone("danger")
        .permission(permissions::PAYROLL_APPROVE)
        .from([PayrollRunStatus::Calculated, PayrollRunStatus::Approved])
        .to(PayrollRunStatus::Draft)
        .reason_required()
        .guard("not_the_preparer", |run: &PayrollRun, ctx| {
            let problem = preparer_problem(run, ctx);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|run: &PayrollRun, ctx, tx| {
            let id = run.id;
            let reason = ctx.reason().map(str::trim).map(str::to_owned);
            Box::pin(async move {
                let commented = PayrollLine::query()
                    .where_(PayrollLine::RUN_ID.eq(id))
                    .where_(PayrollLine::APPROVER_COMMENT.is_not_null())
                    .count(&mut *tx)
                    .await?;
                if commented == 0 {
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        english("wl_hr.payroll.comment_required", &[]),
                    ));
                }
                PayrollRun::query()
                    .where_(PayrollRun::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("approved_by", &None::<i64>),
                            ("return_reason", &reason),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// Sets or clears the approver's comment on line `line_id` (blank clears
/// it). The page route checks who may comment.
pub async fn comment_line(db: &Db, line_id: i64, comment: &str) -> Result<()> {
    let comment = Some(comment.trim().to_owned()).filter(|c| !c.is_empty());
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line_id))
        .update(
            db,
            &[("approver_comment", &comment), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// The next SIF version of run `run_id`: every `included` line goes into it
/// (a held, paid or failed line does not), see [`sif::generate_in`].
async fn generate_sif_in(tx: &mut Transaction, run_id: i64) -> Result {
    sif::generate_in(tx, run_id, None).await
}

/// The bank's result for run `run_id`: the failed lines are `failed` with
/// their reason, except the line of an `absconded` employee, which is `held`
/// with *absence report* and not regenerated; every other line of the file
/// is `paid`.
pub(crate) async fn record_result_in(
    tx: &mut Transaction,
    run_id: i64,
    input: &PartialInput,
) -> Result {
    let lines = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Included))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?;
    for line in lines.into_iter().filter(|l| l.sif_version.is_some()) {
        let failure = input.failed.iter().find(|f| f.line_id == line.id);
        let (status, held, failed) = match failure {
            None => (LineStatus::Paid, None, None),
            Some(failure) => {
                let absconded = Employee::find(&mut *tx, line.employee_id)
                    .await?
                    .is_some_and(|e| e.status == EmployeeStatus::Absconded);
                if absconded {
                    (
                        LineStatus::Held,
                        Some(HELD_ABSENCE_REPORT.to_owned()),
                        Some(failure.reason.trim().to_owned()),
                    )
                } else {
                    (
                        LineStatus::Failed,
                        None,
                        Some(failure.reason.trim().to_owned()),
                    )
                }
            }
        };
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("status", &status.as_str().to_owned()),
                    ("held_reason", &held),
                    ("failure_reason", &failed),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        if status == LineStatus::Paid {
            publish_payslip_in(tx, line.id).await?;
        }
    }
    Ok(())
}

/// Publishes the payslip of line `line_id` to the employee portal, once: a
/// line whose payslip is published keeps the time it was. Decision: the
/// payslip document itself (`DOC.GEN.01`) is made by wl-documents when it
/// provides the contract; here the line records that it is published, which
/// is what the portal and `confirm_paid` read.
pub(crate) async fn publish_payslip_in(tx: &mut Transaction, line_id: i64) -> Result {
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line_id))
        .where_(PayrollLine::PAYSLIP_PUBLISHED_AT.is_null())
        .update(
            &mut *tx,
            &[
                ("payslip_published_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Holds the `included` lines of `employee_id` in runs not yet `submitted`
/// (`draft`, `calculated`, `approved`) with `reason`, inside `tx`. A line in a
/// run that was sent stays. Returns the lines held.
pub async fn hold_open_lines_in(
    tx: &mut Transaction,
    employee_id: i64,
    reason: &str,
) -> Result<Vec<i64>> {
    let lines = PayrollLine::query()
        .where_(PayrollLine::EMPLOYEE_ID.eq(employee_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Included))
        .order_by(PayrollLine::ID)
        .get(&mut *tx)
        .await?;
    let mut held = Vec::new();
    for line in lines {
        let unsent = PayrollRun::find(&mut *tx, line.run_id)
            .await?
            .is_some_and(|run| run.status.is_unsent());
        if !unsent {
            continue;
        }
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("status", &LineStatus::Held.as_str().to_owned()),
                    ("held_reason", &Some(reason.to_owned())),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        held.push(line.id);
    }
    Ok(held)
}

/// A failed line was fixed (its pay account changed): it is `included`
/// again, for the next SIF version.
pub async fn fix_line(db: &Db, line_id: i64) -> Result<()> {
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Failed))
        .update(
            db,
            &[
                ("status", &LineStatus::Included.as_str().to_owned()),
                ("failure_reason", &None::<String>),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Runs `transition` on run `id` as the system.
async fn run_system(
    state: &AppState,
    id: i64,
    transition: &str,
    input: &PartialInput,
) -> std::result::Result<PayrollRun, TransitionError> {
    let run = PayrollRun::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let _outcome = run
        .transition(transition)
        .by_system()
        .input(input)
        .run(state)
        .await?;
    PayrollRun::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)
}

/// Sends run `id`: SIF version 1 with the lines that are `included`.
pub async fn submit(state: &AppState, id: i64) -> std::result::Result<PayrollRun, TransitionError> {
    run_system(state, id, SUBMIT, &PartialInput::default()).await
}

/// Records the bank's result for run `id`: `failed` lines with their reason.
pub async fn record_partial(
    state: &AppState,
    id: i64,
    failed: Vec<FailedLine>,
) -> std::result::Result<PayrollRun, TransitionError> {
    run_system(
        state,
        id,
        RECORD_PARTIAL_BY_SYSTEM,
        &PartialInput {
            failed,
            ..PartialInput::default()
        },
    )
    .await
}

/// A new SIF version of run `id` with only the fixed lines.
pub async fn regenerate(
    state: &AppState,
    id: i64,
) -> std::result::Result<PayrollRun, TransitionError> {
    run_system(state, id, REGENERATE_BY_SYSTEM, &PartialInput::default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_open_until_it_is_sent() {
        for status in [
            PayrollRunStatus::Draft,
            PayrollRunStatus::Calculated,
            PayrollRunStatus::Approved,
        ] {
            assert!(status.is_unsent(), "{status:?}");
        }
        for status in [
            PayrollRunStatus::Submitted,
            PayrollRunStatus::PartiallyRejected,
            PayrollRunStatus::Paid,
        ] {
            assert!(!status.is_unsent(), "{status:?}");
        }
    }

    #[test]
    fn the_three_transitions_are_system_transitions_of_the_spec() {
        // `record_partial` and `regenerate` are user transitions of
        // `payroll.prepare` since BL-HRM-13 ([`crate::wps_partial`]); the
        // system runs their twins, with the same states.
        let machine = PayrollRun::machine();
        let table = [
            (SUBMIT, "approved", "submitted"),
            (RECORD_PARTIAL_BY_SYSTEM, "submitted", "partially_rejected"),
            (REGENERATE_BY_SYSTEM, "partially_rejected", "submitted"),
        ];
        for (name, from, to) in table {
            let transition = machine.find(name).unwrap();
            assert!(transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
    }
}
