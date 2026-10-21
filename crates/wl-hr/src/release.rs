//! Releasing held lines (wl-hr §3 *Payroll line*: hold, released into; §4 *Run
//! types*; §5.6 step 2; §6 `HRM.RUL.12`; issue #208).
//!
//! A line is held because of an open absence case ([`HELD_ABSENCE_CASE`]), a
//! valid absence report ([`HELD_ABSENCE_REPORT`]) or a failed line moved out of
//! the run (a reason the preparer wrote, e.g. *closed account*). When the
//! reason clears the line is released:
//!
//! - **Reason cleared before the run is sent.** The line is `included` again
//!   in that run (a run that is `draft`, `calculated` or `approved`).
//! - **Otherwise it waits.** A run that is sent but not `paid` keeps the line
//!   held, marked with the day the reason cleared
//!   ([`PayrollLine::hold_cleared_on`]); `HRM.RUL.12` takes it when the run is
//!   `paid` (the listener of `confirm_paid` and the daily job [`TASK`]). The
//!   paid run is not edited: only the line's `released_into` is set.
//! - **`HRM.RUL.12` proposes an off-cycle run.** [`propose`] adds a line with
//!   the held line's amounts as they were calculated to the `draft` off-cycle
//!   run of the line's legal entity (one per entity collects them until it is
//!   calculated; there is one if none is open), sets `released_into` of the
//!   held line, and tells the payroll officers (the holders of
//!   `payroll.prepare`). It neither calculates nor submits. A line is
//!   released once: with `released_into` set it is not added again.
//! - **Carrying into the next regular run** ([`carry`]) needs a reason, and
//!   warns when the pay falls after the WPS due date of the line's period
//!   ([`late_warning`]); a line held by a valid absence report carries no
//!   warning, it is outside WPS (MR 340/2026 Art. 4). A line proposed into a
//!   draft off-cycle run can be moved into a regular run this way.
//!
//! What clears a reason: `AbsenceCaseClosed` ([`on_case_closed`]), the report
//! withdrawn at `reinstate` ([`report_withdrawn`]), and a line fixed by the
//! preparer ([`line_fixed`], what the change of a pay account calls too).
//!
//! Decisions: the period of an off-cycle run is the period of the oldest line
//! it holds, the one whose WPS due date is nearest. The job runs daily
//! ([`TASK`]); the cut-off rule of the spec ("at each cut-off for lines still
//! held") is covered because a cleared line is proposed the day it clears and
//! again by every run of the job until it is released.

use renox::auth::permissions::users_with_permission;
use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::workforce::AbsenceCaseClosed;

use crate::config::config_of;
use crate::hours::payroll_period;
use crate::notices;
use crate::payroll::{
    HELD_ABSENCE_CASE, HELD_ABSENCE_REPORT, LineStatus, PayrollLine, PayrollRun, PayrollRunStatus,
};
use crate::permissions;
use crate::texts::english;
use crate::wps_alerts::wps_due_date;

/// The run type of a run that pays released lines.
pub const OFF_CYCLE: &str = "off_cycle";

/// The daily job, and the command, that run [`run`].
pub const TASK: &str = "hr:held-lines";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "06:00";

fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// What a clearing did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cleared {
    /// Lines made `included` again in their unsent run.
    pub unheld: usize,
    /// Lines of a sent or paid run marked as cleared; `HRM.RUL.12` takes them.
    pub waiting: usize,
}

/// What a proposal did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Proposed {
    /// Lines added to an off-cycle run now.
    pub lines: usize,
    /// Off-cycle runs made now.
    pub runs: usize,
}

/// Clears the hold of `line`, inside `tx`: it is `included` again when its run
/// is not sent, else it is marked cleared on `day`. Returns whether it was
/// un-held.
async fn clear_in(tx: &mut Transaction, line: &PayrollLine, day: NaiveDate) -> Result<bool> {
    let unsent = PayrollRun::find(&mut *tx, line.run_id)
        .await?
        .is_some_and(|run| run.status.is_unsent());
    if unsent {
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("status", &LineStatus::Included.as_str().to_owned()),
                    ("held_reason", &None::<String>),
                    ("hold_cleared_on", &None::<NaiveDate>),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    } else {
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .where_(PayrollLine::HOLD_CLEARED_ON.is_null())
            .update(
                &mut *tx,
                &[("hold_cleared_on", &Some(day)), ("updated_at", &now())],
            )
            .await?;
    }
    Ok(unsent)
}

/// The held lines of `employee_id` whose reason is one of `reasons`, that
/// nothing released yet.
async fn held_for(db: &Db, employee_id: i64, reasons: &[&str]) -> Result<Vec<PayrollLine>> {
    Ok(PayrollLine::query()
        .where_(PayrollLine::EMPLOYEE_ID.eq(employee_id))
        .where_(PayrollLine::STATUS.eq(LineStatus::Held))
        .where_(PayrollLine::RELEASED_INTO.is_null())
        .where_(PayrollLine::SOURCE_LINE_ID.is_null())
        .order_by(PayrollLine::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|l| {
            l.held_reason
                .as_deref()
                .is_some_and(|reason| reasons.contains(&reason))
        })
        .collect())
}

/// Clears the holds of `lines` and proposes the off-cycle run for those that
/// are in a paid run.
async fn clear_lines(state: &AppState, lines: Vec<PayrollLine>) -> Result<Cleared> {
    let mut cleared = Cleared::default();
    if lines.is_empty() {
        return Ok(cleared);
    }
    let day = today();
    let mut tx = state.db.begin().await?;
    for line in &lines {
        if clear_in(&mut tx, line, day).await? {
            cleared.unheld += 1;
        } else {
            cleared.waiting += 1;
        }
    }
    tx.commit().await?;
    if cleared.waiting > 0 {
        propose(state).await?;
    }
    Ok(cleared)
}

/// `AbsenceCaseClosed`: the lines held for the employee's open absence case
/// are released. A line held by a valid absence report stays until the report
/// is withdrawn.
pub(crate) async fn on_case_closed(state: &AppState, event: &AbsenceCaseClosed) -> Result {
    let lines = held_for(&state.db, event.employee.0, &[HELD_ABSENCE_CASE]).await?;
    clear_lines(state, lines).await.map(|_| ())
}

/// The MoHRE absence report of `employee_id` was withdrawn (`reinstate`): the
/// lines held by it are released.
pub async fn report_withdrawn(state: &AppState, employee_id: i64) -> Result<Cleared> {
    let lines = held_for(&state.db, employee_id, &[HELD_ABSENCE_REPORT]).await?;
    clear_lines(state, lines).await
}

/// The reason of the hold of line `line_id` is gone (a pay account was fixed,
/// the preparer saw it was settled): the line is released. A line the bank
/// failed is made `included` again for the next SIF version. Refused when the
/// line is neither held nor failed, or is already released.
pub async fn line_fixed(state: &AppState, line_id: i64) -> Result<Cleared> {
    let line = PayrollLine::find_or_404(&state.db, line_id).await?;
    // A failed line is fixed in place: `included` for the next SIF version.
    if line.status == LineStatus::Failed {
        crate::payroll::fix_line(&state.db, line_id).await?;
        return Ok(Cleared {
            unheld: 1,
            waiting: 0,
        });
    }
    if line.status != LineStatus::Held {
        return Err(refuse("wl_hr.payroll.line_not_held"));
    }
    if line.released_into.is_some() {
        return Err(refuse("wl_hr.payroll.already_released"));
    }
    clear_lines(state, vec![line]).await
}

/// The draft off-cycle run of `entity`, made when there is none, with the
/// period `period`. Returns the run and whether it was made.
async fn open_off_cycle_in(
    tx: &mut Transaction,
    entity: Option<i64>,
    period: &str,
) -> Result<(PayrollRun, bool)> {
    let mut query = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq(OFF_CYCLE))
        .where_(PayrollRun::STATUS.eq(PayrollRunStatus::Draft));
    query = match entity {
        Some(entity) => query.where_(PayrollRun::LEGAL_ENTITY_ID.eq(entity)),
        None => query.where_(PayrollRun::LEGAL_ENTITY_ID.is_null()),
    };
    if let Some(run) = query.order_by(PayrollRun::ID).first(&mut *tx).await? {
        return Ok((run, false));
    }
    let mut run = PayrollRun {
        legal_entity_id: entity,
        period: period.to_owned(),
        run_type: OFF_CYCLE.to_owned(),
        ..PayrollRun::default()
    };
    run.insert(&mut *tx).await?;
    Ok((run, true))
}

/// A copy of the held `line` for `run`: its amounts as they were calculated,
/// `included`, linked to the held line.
fn copy_of(line: &PayrollLine, run_id: i64) -> PayrollLine {
    PayrollLine {
        id: 0,
        run_id,
        employee_id: line.employee_id,
        net_amount: line.net_amount,
        fixed_wage: line.fixed_wage,
        variable_amount: line.variable_amount,
        difference_amount: line.difference_amount,
        deductions_amount: line.deductions_amount,
        cost_object_type: line.cost_object_type.clone(),
        cost_object_id: line.cost_object_id,
        status: LineStatus::Included,
        source_line_id: Some(line.id),
        ..PayrollLine::default()
    }
}

/// `HRM.RUL.12`: adds the released lines of paid runs to the draft off-cycle
/// run of their legal entity, and tells the payroll officers. A line already
/// released is not added again.
pub async fn propose(state: &AppState) -> Result<Proposed> {
    let candidates = PayrollLine::query()
        .where_(PayrollLine::STATUS.eq(LineStatus::Held))
        .where_(PayrollLine::HOLD_CLEARED_ON.is_not_null())
        .where_(PayrollLine::RELEASED_INTO.is_null())
        .where_(PayrollLine::SOURCE_LINE_ID.is_null())
        .order_by(PayrollLine::ID)
        .get(&state.db)
        .await?;
    let mut proposed = Proposed::default();
    // The off-cycle runs that got lines now, with how many.
    let mut told: Vec<(i64, usize)> = Vec::new();
    for line in candidates {
        let mut tx = state.db.begin().await?;
        let Some(source) = PayrollRun::find(&mut tx, line.run_id)
            .await?
            .filter(|run| run.status == PayrollRunStatus::Paid)
        else {
            continue;
        };
        let (run, made) =
            open_off_cycle_in(&mut tx, source.legal_entity_id, &source.period).await?;
        let mut copy = copy_of(&line, run.id);
        copy.insert(&mut tx).await?;
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                &mut tx,
                &[("released_into", &Some(run.id)), ("updated_at", &now())],
            )
            .await?;
        // The run carries the period of its oldest line.
        if !made && run.period > source.period {
            PayrollRun::query()
                .where_(PayrollRun::ID.eq(run.id))
                .update(
                    &mut tx,
                    &[("period", &source.period), ("updated_at", &now())],
                )
                .await?;
        }
        tx.commit().await?;
        proposed.lines += 1;
        proposed.runs += usize::from(made);
        match told.iter_mut().find(|(id, _)| *id == run.id) {
            Some((_, count)) => *count += 1,
            None => told.push((run.id, 1)),
        }
    }
    if !told.is_empty() {
        let officers: Vec<i64> = users_with_permission(&state.db, permissions::PAYROLL_PREPARE)
            .await?
            .iter()
            .map(|u| u.id)
            .collect();
        for (run_id, count) in told {
            if let Some(run) = PayrollRun::find(&state.db, run_id).await? {
                notices::payroll_released(state, &run, count, &officers).await?;
            }
        }
    }
    Ok(proposed)
}

/// The warning that carrying a line held for `reason`, from period `held_in`
/// into the regular run of `into` (`2026-12`), pays after the WPS due date of
/// `held_in`, with `cut_off_day` the day the run is paid from and `today` the
/// day it is carried. `None` when it does not, and for a line held by a valid
/// absence report: it is outside WPS.
pub fn late_warning(
    reason: Option<&str>,
    held_in: &str,
    into: &str,
    cut_off_day: u32,
    today: NaiveDate,
) -> Option<String> {
    if reason == Some(HELD_ABSENCE_REPORT) {
        return None;
    }
    let due = wps_due_date(held_in)?;
    let paid_from = payroll_period(into, cut_off_day)?.cut_off.max(today);
    (paid_from > due).then(|| english("wl_hr.payroll.late_warning", &[("due", &due)]))
}

/// What carrying a line did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    /// The line added to the regular run.
    pub line_id: i64,
    /// The warning that it pays after the WPS due date, if it does.
    pub warning: Option<String>,
}

/// The warning for carrying held line `line` into the regular run `target`
/// (what the run page shows next to the reason).
pub async fn warning_for(
    state: &AppState,
    line: &PayrollLine,
    target: &PayrollRun,
) -> Result<Option<String>> {
    let Some(run) = PayrollRun::find(&state.db, line.run_id).await? else {
        return Ok(None);
    };
    Ok(late_warning(
        line.held_reason.as_deref(),
        &run.period,
        &target.period,
        config_of(state).cut_off_day,
        today(),
    ))
}

/// Carries the released held line `line_id` into the `regular` run `run_id`
/// that is still being prepared (`draft` or `calculated`), with a `reason`.
/// The line has to be of the same legal entity, from an earlier period, and
/// either not released yet or proposed into a draft off-cycle run, which it
/// leaves. Returns the new line and the late-payment warning.
pub async fn carry(state: &AppState, line_id: i64, run_id: i64, reason: &str) -> Result<Carried> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refuse("wl_hr.payroll.carry_reason_required"));
    }
    let line = PayrollLine::find_or_404(&state.db, line_id).await?;
    let target = PayrollRun::find_or_404(&state.db, run_id).await?;
    let source = PayrollRun::find_or_404(&state.db, line.run_id).await?;
    if line.status != LineStatus::Held
        || line.hold_cleared_on.is_none()
        || line.source_line_id.is_some()
        || source.status != PayrollRunStatus::Paid
    {
        return Err(refuse("wl_hr.payroll.line_not_releasable"));
    }
    if target.run_type != crate::calculation::REGULAR
        || !matches!(
            target.status,
            PayrollRunStatus::Draft | PayrollRunStatus::Calculated
        )
        || target.legal_entity_id != source.legal_entity_id
        || target.period <= source.period
    {
        return Err(refuse("wl_hr.payroll.carry_target_invalid"));
    }
    // Read before the transaction opens: it holds the write lock.
    let warning = warning_for(state, &line, &target).await?;
    let mut tx = state.db.begin().await?;
    if let Some(released) = line.released_into {
        // Only a line still waiting in a draft off-cycle run can move.
        let waiting = PayrollRun::find(&mut tx, released)
            .await?
            .filter(|r| r.run_type == OFF_CYCLE && r.status == PayrollRunStatus::Draft);
        if waiting.is_none() {
            return Err(refuse("wl_hr.payroll.already_released"));
        }
        PayrollLine::query()
            .where_(PayrollLine::RUN_ID.eq(released))
            .where_(PayrollLine::SOURCE_LINE_ID.eq(line.id))
            .delete(&mut tx)
            .await?;
    }
    let mut copy = copy_of(&line, target.id);
    copy.carry_reason = Some(reason.to_owned());
    copy.late_warning = warning.clone();
    copy.insert(&mut tx).await?;
    PayrollLine::query()
        .where_(PayrollLine::ID.eq(line.id))
        .update(
            &mut tx,
            &[("released_into", &Some(target.id)), ("updated_at", &now())],
        )
        .await?;
    tx.commit().await?;
    Ok(Carried {
        line_id: copy.id,
        warning,
    })
}

/// The held lines a regular run of the same legal entity could carry: cleared
/// and not released into a run that is past `draft`.
pub async fn carryable(state: &AppState, target: &PayrollRun) -> Result<Vec<PayrollLine>> {
    if target.run_type != crate::calculation::REGULAR
        || !matches!(
            target.status,
            PayrollRunStatus::Draft | PayrollRunStatus::Calculated
        )
    {
        return Ok(Vec::new());
    }
    let candidates = PayrollLine::query()
        .where_(PayrollLine::STATUS.eq(LineStatus::Held))
        .where_(PayrollLine::HOLD_CLEARED_ON.is_not_null())
        .where_(PayrollLine::SOURCE_LINE_ID.is_null())
        .order_by(PayrollLine::ID)
        .get(&state.db)
        .await?;
    let mut lines = Vec::new();
    for line in candidates {
        let Some(source) = PayrollRun::find(&state.db, line.run_id).await? else {
            continue;
        };
        let movable = match line.released_into {
            None => true,
            Some(run_id) => PayrollRun::find(&state.db, run_id)
                .await?
                .is_some_and(|r| r.run_type == OFF_CYCLE && r.status == PayrollRunStatus::Draft),
        };
        if movable
            && source.status == PayrollRunStatus::Paid
            && source.legal_entity_id == target.legal_entity_id
            && source.period < target.period
        {
            lines.push(line);
        }
    }
    Ok(lines)
}

/// What a run of the job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Lines added to off-cycle runs.
    pub lines: usize,
    /// Off-cycle runs made.
    pub runs: usize,
}

/// Runs `HRM.RUL.12` once: proposes the off-cycle runs for the lines that were
/// released and are not in a run yet.
pub async fn run(state: &AppState) -> Result<Report> {
    let proposed = propose(state).await?;
    Ok(Report {
        lines: proposed.lines,
        runs: proposed.runs,
    })
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(crate::wps_alerts::TASK_ZONE);
    app.command(
        TASK,
        "Propose the off-cycle run for held payroll lines whose reason has cleared",
        |_args, state: AppState| async move {
            let report = run(&state).await?;
            println!(
                "{} lines added to off-cycle runs, {} runs made.",
                report.lines, report.runs
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn carrying_a_november_line_into_december_pays_after_the_due_date() {
        let warning = late_warning(
            Some("closed account"),
            "2026-11",
            "2026-12",
            25,
            day(2026, 12, 3),
        );
        assert_eq!(
            warning.as_deref(),
            Some("This pays after the WPS due date 2026-12-01.")
        );
    }

    #[test]
    fn a_line_held_by_a_valid_absence_report_carries_no_warning() {
        assert_eq!(
            late_warning(
                Some(HELD_ABSENCE_REPORT),
                "2026-11",
                "2026-12",
                25,
                day(2026, 12, 3)
            ),
            None
        );
    }

    #[test]
    fn no_warning_when_the_run_pays_before_the_due_date() {
        // A November line carried on 10 Nov into a run paid from its cut-off
        // on the 25th is paid before the WPS due date, 1 Dec.
        assert_eq!(
            late_warning(None, "2026-11", "2026-11", 25, day(2026, 11, 10)),
            None
        );
    }
}
