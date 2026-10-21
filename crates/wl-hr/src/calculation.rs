//! The calculation of a `regular` payroll run's lines (wl-hr §4 *Payroll run*
//! `calculate` and `submit`, §5.3 steps 2 and 3, issue #206).
//!
//! `calculate` makes one line for each employee of the run's legal entity
//! who is paid (not `freelance`; `active`, `on_leave`, `notice` or
//! `absconded`) and has a salary structure effective in the period:
//!
//! | Part of the line | From |
//! |---|---|
//! | `fixed_wage` | the structure effective on the last day of the period: basic plus the fixed allowances; the basis of the Art. 25 caps |
//! | `hours_minutes`, `provisional_minutes` | `Timesheets::hours_for_payroll(period)`: approved hours and, for a timesheet the client has not approved, the accepted attendance hours up to the cut-off marked *provisional* |
//! | `variable_amount` | the hours that are not `normal` (overtime, night, rest day, public holiday) at their rate on the hourly rate ([`crate::hours`]); `normal` hours are inside the fixed wage |
//! | `difference_amount` | the hours of adjustment timesheets and reconciliations ([`crate::differences`]), priced with the structure effective on the day of the hours: + paid, - recovered (shown signed; only the paid ones add to the net) |
//! | `deductions_amount` | the adjustments the run carries, among them the recovery of each negative difference ([`crate::caps`] checks it) |
//! | `net_amount` | `fixed_wage + variable_amount + paid differences - deductions_amount` |
//!
//! Decisions:
//!
//! - **Hours come in as input.** A transition's effect has only the
//!   transaction, but the timesheets are asked through the kernel contract
//!   with the app state. [`calculate`] asks them and runs the transition
//!   with the answer as its input; the page route of `calculate` does
//!   the same ([`crate::pages`]).
//! - **A line the calculation does not know stays.** An employee without a
//!   salary structure keeps the line the run holds, as it was entered; the
//!   calculation neither creates nor changes it.
//! - **Missing timesheet.** An employee with no hours does not block the
//!   run: the line carries the exception [`MISSING_TIMESHEET`] for the
//!   preparer to review (§5.3 step 3).
//! - **Holds.** A line of an `absconded` employee is held with *absence
//!   report*, one of an employee with an open absence case with *absence
//!   case* ([`HELD_ABSENCE_CASE`]); a hold set by something else stays until
//!   [`crate::release`] clears it.
//! - **Not in this issue.** Pro-rating of joiners and leavers, unpaid leave
//!   and expense reimbursements.
//! - **`submit`.** The hours of the lines that go into the salary file are
//!   told to the timesheets after the commit ([`record_use`]): approved
//!   timesheets are locked, those paid on provisional hours get
//!   `paid_provisionally_by`, and an adjustment timesheet is locked with
//!   `paid_by`.

use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use serde::{Deserialize, Serialize};
use wl_kernel::EmployeeId;
use wl_kernel::Subject;
use wl_kernel::timesheets::{HoursLine, PayrollHours, timesheets};

use crate::absence::AbsenceCase;
use crate::adjustment::{Adjustment, AdjustmentKind};
use crate::config::config_of;
use crate::differences::{self, HourDifference};
use crate::employee::{Employee, EmployeeStatus, EmploymentType};
use crate::hours::{self, NORMAL};
use crate::payroll::{HELD_ABSENCE_CASE, HELD_ABSENCE_REPORT, LineStatus, PayrollLine, PayrollRun};
use crate::salary::SalaryStructure;
use crate::texts::english;

/// The exception of a line whose employee has no hours in the period.
pub const MISSING_TIMESHEET: &str = "missing_timesheet";

/// The `regular` run type.
pub const REGULAR: &str = "regular";

/// What `calculate` is given: the hours the timesheets give for the period.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct CalculateInput {
    /// The answer of `Timesheets::hours_for_payroll`.
    pub hours: Vec<PayrollHours>,
}

/// The timesheet hours of `run`'s period, for the input of `calculate`. A run
/// that is not `regular`, or whose period is not a month, asks for none.
pub async fn input_for(state: &AppState, run: &PayrollRun) -> Result<CalculateInput> {
    if run.run_type != REGULAR {
        return Ok(CalculateInput::default());
    }
    let Some(period) = hours::payroll_period(&run.period, config_of(state).cut_off_day) else {
        return Ok(CalculateInput::default());
    };
    let hours = timesheets(state).hours_for_payroll(state, &period).await?;
    Ok(CalculateInput { hours })
}

/// One hour row the run was calculated from, with the timesheet's own answer.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_payroll_hours", index(run_id))]
pub struct PayrollHour {
    /// The primary key.
    pub id: i64,
    /// The run.
    #[model(references = "wl_hr_payroll_runs")]
    pub run_id: i64,
    /// The line.
    #[model(references = "wl_hr_payroll_lines")]
    pub line_id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The timesheet's subject type.
    pub timesheet_type: String,
    /// The timesheet's id.
    pub timesheet_id: i64,
    /// Whether the hours are provisional.
    pub provisional: bool,
    /// The minutes of all categories.
    pub minutes: i64,
    /// The hours as the timesheets gave them (JSON).
    pub payload: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Runs `calculate` on run `id` as `user`: the timesheets are asked and the
/// transition runs with their hours.
pub async fn calculate(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<PayrollRun, renox::state_machine::TransitionError> {
    use renox::state_machine::TransitionError;
    let run = PayrollRun::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let input = input_for(state, &run)
        .await
        .map_err(TransitionError::Failed)?;
    let _outcome = run
        .transition(crate::payroll::CALCULATE)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    PayrollRun::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)
}

/// The statuses of employees who are paid.
const PAID: [EmployeeStatus; 4] = [
    EmployeeStatus::Active,
    EmployeeStatus::OnLeave,
    EmployeeStatus::Notice,
    EmployeeStatus::Absconded,
];

/// Makes or updates the lines of a `regular` run from `input`, inside `tx`.
/// Returns the ids of the lines it calculated.
pub(crate) async fn lines_in(
    tx: &mut Transaction,
    run: &PayrollRun,
    input: &CalculateInput,
) -> Result<Vec<i64>> {
    if run.run_type != REGULAR {
        return Ok(Vec::new());
    }
    let Some((_, to)) = hours::period_bounds(&run.period) else {
        return Ok(Vec::new());
    };
    let mut employees = Employee::query()
        .where_(Employee::STATUS.is_in(PAID))
        .where_(Employee::EMPLOYMENT_TYPE.ne(EmploymentType::Freelance));
    employees = match run.legal_entity_id {
        Some(entity) => employees.where_(Employee::LEGAL_ENTITY_ID.eq(entity)),
        None => employees.where_(Employee::LEGAL_ENTITY_ID.is_null()),
    };
    let employees = employees.order_by(Employee::ID).get(&mut *tx).await?;
    PayrollHour::query()
        .where_(PayrollHour::RUN_ID.eq(run.id))
        .delete(&mut *tx)
        .await?;
    let mut calculated = Vec::new();
    for employee in employees {
        let Some(structure) = SalaryStructure::effective_in(&mut *tx, employee.id, to).await?
        else {
            continue;
        };
        let line = line_of(&mut *tx, run, &employee, &structure, input).await?;
        calculated.push(line);
    }
    Ok(calculated)
}

/// The line of `employee` in `run`, calculated.
async fn line_of(
    tx: &mut Transaction,
    run: &PayrollRun,
    employee: &Employee,
    structure: &SalaryStructure,
    input: &CalculateInput,
) -> Result<i64> {
    let given: Vec<&PayrollHours> = input
        .hours
        .iter()
        .filter(|h| h.owner == EmployeeId(employee.id))
        .collect();
    let minutes: i64 = given.iter().map(|h| h.total_minutes()).sum();
    let provisional: i64 = given
        .iter()
        .filter(|h| h.provisional)
        .map(|h| h.total_minutes())
        .sum();
    let hourly = structure.hourly();
    let variable: i64 = given
        .iter()
        .flat_map(|h| h.totals())
        .filter(|t| t.category != NORMAL)
        .map(|t| hours::amount_for(t.minutes, hourly, &t.category))
        .sum();
    let (paid, recovered) = settle_differences(&mut *tx, run, employee.id, structure).await?;

    let existing = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run.id))
        .where_(PayrollLine::EMPLOYEE_ID.eq(employee.id))
        .where_(PayrollLine::SOURCE_LINE_ID.is_null())
        .order_by(PayrollLine::ID)
        .first(&mut *tx)
        .await?;
    let open_case = AbsenceCase::query()
        .where_(AbsenceCase::EMPLOYEE_ID.eq(employee.id))
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .exists(&mut *tx)
        .await?;
    let (status, held_reason) = hold_of(existing.as_ref(), employee, open_case);
    let mut line = existing.unwrap_or_else(|| PayrollLine {
        run_id: run.id,
        employee_id: employee.id,
        ..PayrollLine::default()
    });
    line.fixed_wage = structure.fixed_wage();
    line.variable_amount = variable;
    line.difference_amount = paid - recovered;
    line.hours_minutes = minutes;
    line.provisional_minutes = provisional;
    line.exception = given.is_empty().then(|| MISSING_TIMESHEET.to_owned());
    line.status = status;
    line.held_reason = held_reason;
    line.save(&mut *tx).await?;
    for h in given {
        let mut row = PayrollHour {
            run_id: run.id,
            line_id: line.id,
            employee_id: employee.id,
            timesheet_type: h.timesheet.subject_type.clone(),
            timesheet_id: h.timesheet.subject_id,
            provisional: h.provisional,
            minutes: h.total_minutes(),
            payload: renox::serde_json::to_string(h).unwrap_or_default(),
            ..PayrollHour::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(line.id)
}

/// Whether the line is `included` or `held`, and why. A hold the calculation
/// does not set (a failed line moved out, a hold of an exit) stays; the hold
/// of an absence case ends with the case.
fn hold_of(
    existing: Option<&PayrollLine>,
    employee: &Employee,
    open_case: bool,
) -> (LineStatus, Option<String>) {
    if employee.status == EmployeeStatus::Absconded {
        return (LineStatus::Held, Some(HELD_ABSENCE_REPORT.to_owned()));
    }
    if let Some(line) = existing.filter(|l| l.status == LineStatus::Held) {
        let case_hold = line.held_reason.as_deref() == Some(HELD_ABSENCE_CASE);
        if !case_hold || open_case {
            return (LineStatus::Held, line.held_reason.clone());
        }
        return (LineStatus::Included, None);
    }
    if open_case {
        return (LineStatus::Held, Some(HELD_ABSENCE_CASE.to_owned()));
    }
    (LineStatus::Included, None)
}

/// Prices the open differences of `employee_id` that no other run took, links
/// them to `run`, and keeps the recovery of each negative one as an
/// adjustment. Returns what is paid and what is recovered, in fils (both
/// positive).
async fn settle_differences(
    tx: &mut Transaction,
    run: &PayrollRun,
    employee_id: i64,
    current: &SalaryStructure,
) -> Result<(i64, i64)> {
    let open = HourDifference::query()
        .where_(HourDifference::EMPLOYEE_ID.eq(employee_id))
        .where_(HourDifference::STATUS.eq(differences::OPEN))
        .order_by(HourDifference::ID)
        .get(&mut *tx)
        .await?;
    let (mut paid, mut recovered) = (0, 0);
    for difference in open
        .into_iter()
        .filter(|d| d.run_id.is_none() || d.run_id == Some(run.id))
    {
        let structure = differences::structure_for(&mut *tx, &difference, current).await?;
        let amount =
            hours::amount_for(difference.minutes, structure.hourly(), &difference.category);
        HourDifference::query()
            .where_(HourDifference::ID.eq(difference.id))
            .update(
                &mut *tx,
                &[
                    ("amount", &amount),
                    ("run_id", &Some(run.id)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        let recovery = Adjustment::query()
            .where_(Adjustment::DIFFERENCE_ID.eq(difference.id))
            .first(&mut *tx)
            .await?;
        if amount < 0 {
            recovered += -amount;
            match recovery {
                Some(_) => {
                    Adjustment::query()
                        .where_(Adjustment::DIFFERENCE_ID.eq(difference.id))
                        .update(&mut *tx, &[("amount", &-amount), ("updated_at", &now())])
                        .await?;
                }
                None => {
                    let note = english("wl_hr.payroll.difference_note", &[]);
                    let mut adjustment = Adjustment {
                        employee_id,
                        kind: AdjustmentKind::OverpaymentRecovery,
                        amount: -amount,
                        period: run.period.clone(),
                        difference_id: Some(difference.id),
                        note: Some(note),
                        ..Adjustment::default()
                    };
                    adjustment.insert(&mut *tx).await?;
                }
            }
        } else {
            paid += amount;
            if recovery.is_some() {
                Adjustment::query()
                    .where_(Adjustment::DIFFERENCE_ID.eq(difference.id))
                    .delete(&mut *tx)
                    .await?;
            }
        }
    }
    Ok((paid, recovered))
}

/// Sets the deductions and the net of the lines `ids`, once the adjustments of
/// the run are linked to them.
pub(crate) async fn settle_in(tx: &mut Transaction, run_id: i64, ids: &[i64]) -> Result {
    for id in ids {
        let Some(line) = PayrollLine::find(&mut *tx, *id).await? else {
            continue;
        };
        let deductions: i64 = Adjustment::query()
            .where_(Adjustment::RUN_ID.eq(run_id))
            .where_(Adjustment::EMPLOYEE_ID.eq(line.employee_id))
            .get(&mut *tx)
            .await?
            .iter()
            .map(|a| a.amount)
            .sum();
        // The recoveries of negative differences are among the deductions;
        // only the differences that are paid add to the net.
        let paid: i64 = HourDifference::query()
            .where_(HourDifference::RUN_ID.eq(run_id))
            .where_(HourDifference::EMPLOYEE_ID.eq(line.employee_id))
            .where_(HourDifference::STATUS.eq(differences::OPEN))
            .where_(HourDifference::AMOUNT.gt(0_i64))
            .get(&mut *tx)
            .await?
            .iter()
            .map(|d| d.amount)
            .sum();
        let net = line.fixed_wage + line.variable_amount + paid - deductions;
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("deductions_amount", &deductions),
                    ("net_amount", &net),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// After `submit`: tells the timesheets which hours the salary file paid.
pub(crate) async fn record_use(state: &AppState, run_id: i64) -> Result {
    let Some(run) = PayrollRun::find(&state.db, run_id).await? else {
        return Ok(());
    };
    let lines = run.lines(&state.db).await?;
    let sent: Vec<i64> = lines
        .iter()
        .filter(|l| l.sif_version.is_some())
        .map(|l| l.id)
        .collect();
    let mut used: Vec<PayrollHours> = Vec::new();
    for row in PayrollHour::query()
        .where_(PayrollHour::RUN_ID.eq(run_id))
        .order_by(PayrollHour::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|row| sent.contains(&row.line_id))
    {
        if let Ok(hours) = renox::serde_json::from_str::<PayrollHours>(&row.payload) {
            used.push(hours);
        }
    }
    let employees: Vec<(i64, i64)> = lines
        .iter()
        .filter(|l| sent.contains(&l.id))
        .map(|l| (l.employee_id, l.id))
        .collect();
    for difference in HourDifference::of_run(&state.db, run_id).await? {
        if difference.status != differences::OPEN
            || !employees.iter().any(|(e, _)| *e == difference.employee_id)
        {
            continue;
        }
        let line = HoursLine {
            date: difference.work_date,
            category: difference.category.clone(),
            minutes: difference.minutes,
        };
        let timesheet = difference.timesheet();
        match used
            .iter_mut()
            .find(|h| h.timesheet == timesheet && h.owner == EmployeeId(difference.employee_id))
        {
            Some(hours) => hours.lines.push(line),
            None => used.push(PayrollHours {
                timesheet,
                owner: EmployeeId(difference.employee_id),
                period_from: difference.work_date,
                period_to: difference.work_date,
                context: None,
                approver_kind: "internal".to_owned(),
                provisional: false,
                lines: vec![line],
            }),
        }
    }
    if used.is_empty() {
        return Ok(());
    }
    let subject: Subject = run.subject();
    timesheets(state)
        .record_payroll_use(state, &subject, &used)
        .await
}
