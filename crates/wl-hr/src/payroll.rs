//! Payroll runs and lines as far as an exit needs them (wl-hr §4 *Payroll
//! run*, §5.6, issue #197). The lifecycle of a run (`calculate`, `approve`,
//! `return`, `reject`, `confirm_paid`, ...) comes with BL-HRM-11 to 18; here
//! are the run and its lines, and the three transitions that decide which
//! line goes into which SIF file:
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
//! The three are `system` transitions run by [`submit`], [`record_partial`]
//! and [`regenerate`]; the user-facing ones, with `payroll.prepare`, replace
//! them with BL-HRM-11 to 14 (decision: this issue builds the base only as
//! far as the held lines need).
//!
//! [`hold_open_lines_in`] is what the exits use: the lines that are
//! `included` in runs not yet `submitted` (`draft`, `calculated`, `approved`)
//! become `held` with a reason; a line in a file already sent stays.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};

use crate::employee::{Employee, EmployeeStatus};
use crate::texts::english;

/// The reason of a line held by the absence report.
pub const HELD_ABSENCE_REPORT: &str = "absence report";

/// The run is ready to be sent.
pub const SUBMIT: &str = "submit";
/// The bank's result file shows failed lines.
pub const RECORD_PARTIAL: &str = "record_partial";
/// A new SIF file with the fixed lines.
pub const REGENERATE: &str = "regenerate";

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
    index(employee_id, status)
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
    /// Where it stands.
    pub status: LineStatus,
    /// Why it is held.
    pub held_reason: Option<String>,
    /// Why the bank failed it.
    pub failure_reason: Option<String>,
    /// The SIF version that carries it; empty when it is in none.
    pub sif_version: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayrollRun {
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
}

impl StateMachine for PayrollRun {
    type State = PayrollRunStatus;
    const COLUMN: renox::db::Col<PayrollRun, PayrollRunStatus> = PayrollRun::STATUS;

    fn machine() -> Machine<Self> {
        use PayrollRunStatus::{Approved, Calculated, Draft, Paid, PartiallyRejected, Submitted};
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
            .state(PartiallyRejected)
            .tone("warning")
            .state(Paid)
            .tone("success")
            .terminal()
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
            .transition(
                Transition::new(RECORD_PARTIAL)
                    .label("Record partial rejection")
                    .system()
                    .from([Submitted])
                    .to(PartiallyRejected)
                    .guard("reasons_given", |_run: &PayrollRun, ctx| {
                        let input = ctx.input_as::<PartialInput>().unwrap_or_default();
                        let blank = input.failed.iter().any(|f| f.reason.trim().is_empty());
                        Box::pin(async move {
                            if blank {
                                Err(english("wl_hr.payroll.reason_required", &[]))
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|run: &PayrollRun, ctx, tx| {
                        let id = run.id;
                        let input = ctx.input_as::<PartialInput>().unwrap_or_default();
                        Box::pin(async move { record_result_in(tx, id, &input).await })
                    }),
            )
            .transition(
                Transition::new(REGENERATE)
                    .label("Regenerate")
                    .system()
                    .from([PartiallyRejected])
                    .to(Submitted)
                    .effect(|run: &PayrollRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move {
                            let failed = PayrollLine::query()
                                .where_(PayrollLine::RUN_ID.eq(id))
                                .where_(PayrollLine::STATUS.eq(LineStatus::Failed))
                                .count(&mut *tx)
                                .await?;
                            if failed > 0 {
                                return Err(Error::Status(
                                    StatusCode::UNPROCESSABLE_ENTITY,
                                    english("wl_hr.payroll.failed_lines_left", &[]),
                                ));
                            }
                            generate_sif_in(tx, id).await
                        })
                    }),
            )
    }
}

/// The next SIF version of run `run_id`: every `included` line goes into it
/// (a held, paid or failed line does not).
async fn generate_sif_in(tx: &mut Transaction, run_id: i64) -> Result {
    let Some(run) = PayrollRun::find(&mut *tx, run_id).await? else {
        return Ok(());
    };
    let version = run.sif_version + 1;
    PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Included))
        .update(
            &mut *tx,
            &[("sif_version", &Some(version)), ("updated_at", &now())],
        )
        .await?;
    PayrollRun::query()
        .where_(PayrollRun::ID.eq(run_id))
        .update(
            &mut *tx,
            &[("sif_version", &version), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// The bank's result for run `run_id`: the failed lines are `failed` with
/// their reason, except the line of an `absconded` employee, which is `held`
/// with *absence report* and not regenerated; every other line of the file
/// is `paid`.
async fn record_result_in(tx: &mut Transaction, run_id: i64, input: &PartialInput) -> Result {
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
    }
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
    run_system(state, id, RECORD_PARTIAL, &PartialInput { failed }).await
}

/// A new SIF version of run `id` with only the fixed lines.
pub async fn regenerate(
    state: &AppState,
    id: i64,
) -> std::result::Result<PayrollRun, TransitionError> {
    run_system(state, id, REGENERATE, &PartialInput::default()).await
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
        let machine = PayrollRun::machine();
        let table = [
            (SUBMIT, "approved", "submitted"),
            (RECORD_PARTIAL, "submitted", "partially_rejected"),
            (REGENERATE, "partially_rejected", "submitted"),
        ];
        for (name, from, to) in table {
            let transition = machine.find(name).unwrap();
            assert!(transition.is_system(), "{name}");
            assert_eq!(transition.from_states(), [from], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
    }
}
