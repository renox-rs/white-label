//! Submitting a timesheet (wl-attendance §4 `submit`, §5.4, §5.5 point 4;
//! BL-ATT-06): the hours of one owner and period go to the approver.
//!
//! For a timesheet whose hours come from attendance (02, 09; not manual or
//! generated hours, not an adjustment) `submit` is refused, by the machine
//! and so for a user and for the system alike, unless:
//!
//! - **No attendance is left `flagged`** in the period (a held record
//!   proposes no hours, [`crate::proposal`]).
//! - **Every attended day has hours allocated to a cost object.** A day with
//!   an `accepted` or `locked` record of the owner needs entries with a cost
//!   object, on this timesheet or on another regular timesheet of the owner.
//! - **The hours on a day do not exceed the attended hours.** The hours of
//!   the owner on a day, summed over this timesheet and every other regular
//!   timesheet that has been submitted (or is approved or locked), are
//!   compared in minutes with the worked minutes of the `accepted` and
//!   `locked` records of that day. The refusal names each day and the
//!   excess. Nothing is trimmed.
//!
//! Decisions:
//!
//! - Another timesheet counts once it has been submitted: two timesheets
//!   that are both `open` do not hold each other's hours, so the first one
//!   submitted wins and the second is refused with the excess. A rejected
//!   timesheet (back to `open`) stops counting.
//! - The check and the transition of a submit run while the lock of each
//!   day of the timesheet's entries is held ([`crate::lock`], the same lock
//!   as entry saves and records), taken in date order, so that two submits
//!   of the same employee and day see each other's entries.
//! - A system submit at period end ([`submit_by_system`]) runs the same
//!   checks. When one fails the timesheet stays `open` and the submitter
//!   (the user who created it, else the owner's user) gets a task listing the
//!   days, through the kernel's task creator; without ticketing no task is
//!   opened.
//! - The notification of the approver is part of the approval issue.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::state_machine::{Transition, TransitionError};
use wl_kernel::BusinessCalendar;
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY, TicketRef};
use wl_kernel::workforce::workforce;

use crate::access;
use crate::entry::{TimesheetEntry, hours_text};
use crate::location::WorkLocation;
use crate::lock;
use crate::overtime::worked_minutes;
use crate::permissions;
use crate::record::{AttendanceRecord, AttendanceStatus};
use crate::service::refuse;
use crate::texts::english;
use crate::timesheet::{HoursSource, Timesheet, TimesheetKind, TimesheetStatus};

/// A user submits an open timesheet (`timesheets.submit`).
pub const SUBMIT: &str = "submit";
/// The system submits an open timesheet at period end (`HRM.RUL.08`).
pub const SUBMIT_BY_SYSTEM: &str = "submit_by_system";

/// The transitions of `submit`, for the timesheet's machine.
pub(crate) fn transitions() -> Vec<Transition<Timesheet>> {
    vec![
        Transition::new(SUBMIT)
            .label("Submit")
            .tone("primary")
            .from([TimesheetStatus::Open])
            .to(TimesheetStatus::Submitted)
            .guard("dispute_not_upheld", |sheet: &Timesheet, _ctx| {
                // An adjustment of a client dispute that ended `upheld` has
                // no payroll or billing effect and is not submitted
                // (`crate::dispute`, BL-ATT-09).
                let upheld = sheet.dispute_upheld_at.is_some();
                Box::pin(async move {
                    if upheld {
                        Err(english("wl_attendance.adjustment.dispute_upheld", &[]))
                    } else {
                        Ok(())
                    }
                })
            })
            .effect(|sheet: &Timesheet, ctx, tx| {
                let sheet = sheet.clone();
                let by = ctx.actor().map(|user| user.id);
                Box::pin(async move { submit_in(tx, &sheet, by).await })
            }),
        Transition::new(SUBMIT_BY_SYSTEM)
            .label("Submit")
            .system()
            .from([TimesheetStatus::Open])
            .to(TimesheetStatus::Submitted)
            .guard("dispute_not_upheld", |sheet: &Timesheet, _ctx| {
                // An adjustment of a client dispute that ended `upheld` has
                // no payroll or billing effect and is not submitted
                // (`crate::dispute`, BL-ATT-09).
                let upheld = sheet.dispute_upheld_at.is_some();
                Box::pin(async move {
                    if upheld {
                        Err(english("wl_attendance.adjustment.dispute_upheld", &[]))
                    } else {
                        Ok(())
                    }
                })
            })
            .effect(|sheet: &Timesheet, _ctx, tx| {
                let sheet = sheet.clone();
                Box::pin(async move { submit_in(tx, &sheet, None).await })
            }),
    ]
}

/// The effect of both: the guards, then who submitted and when.
async fn submit_in(tx: &mut Transaction, sheet: &Timesheet, by: Option<i64>) -> Result {
    let problems = problems_in(tx, sheet).await?;
    if !problems.is_empty() {
        return Err(refuse(problems.join(" ")));
    }
    Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .update(
            &mut *tx,
            &[
                ("submitted_by", &by),
                ("submitted_at", &Some(now())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Why `sheet` cannot be submitted, one sentence per problem; empty when it
/// can. Nothing is written.
///
/// # Errors
///
/// The database's errors.
pub async fn problems_in(tx: &mut Transaction, sheet: &Timesheet) -> Result<Vec<String>> {
    if sheet.kind == TimesheetKind::Adjustment || sheet.hours_source != HoursSource::Attendance {
        return Ok(Vec::new());
    }
    // Only the time zone of the calendar is used here.
    let calendar = BusinessCalendar::default();
    let owner = sheet.owner_employee_id;
    let mut problems = Vec::new();

    // No attendance left flagged.
    let flagged = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(owner))
        .where_(AttendanceRecord::STATUS.eq(AttendanceStatus::Flagged))
        .where_(AttendanceRecord::DATE.gte(sheet.period_from))
        .where_(AttendanceRecord::DATE.lte(sheet.period_to))
        .order_by(AttendanceRecord::DATE)
        .order_by(AttendanceRecord::ID)
        .get(&mut *tx)
        .await?;
    for record in &flagged {
        let location = WorkLocation::find(&mut *tx, record.location_id)
            .await?
            .map_or_else(|| format!("#{}", record.location_id), |l| l.name);
        let flags = crate::flags::AttendanceFlag::of_record_in(tx, record.id)
            .await?
            .into_iter()
            .map(|f| f.flag_type.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(english(
            "wl_attendance.submit.flagged_record",
            &[
                ("record", &record.id),
                ("date", &record.date),
                ("location", &location),
                ("flags", &flags),
            ],
        ));
    }

    // The attended minutes per day.
    let attended_records = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(owner))
        .where_(
            AttendanceRecord::STATUS.is_in([AttendanceStatus::Accepted, AttendanceStatus::Locked]),
        )
        .where_(AttendanceRecord::DATE.gte(sheet.period_from))
        .where_(AttendanceRecord::DATE.lte(sheet.period_to))
        .get(&mut *tx)
        .await?;
    let mut attended: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    for record in &attended_records {
        *attended.entry(record.date).or_default() += worked_minutes(&calendar, record);
    }

    // The other regular timesheets of the owner.
    let others: Vec<Timesheet> = Timesheet::query()
        .where_(Timesheet::OWNER_EMPLOYEE_ID.eq(owner))
        .where_(Timesheet::KIND.eq(TimesheetKind::Regular))
        .where_(Timesheet::ID.ne(sheet.id))
        .get(&mut *tx)
        .await?;
    let submitted_ids: BTreeSet<i64> = others
        .iter()
        .filter(|t| t.status != TimesheetStatus::Open)
        .map(|t| t.id)
        .collect();
    let other_ids: Vec<i64> = others.iter().map(|t| t.id).collect();

    let own = TimesheetEntry::query()
        .where_(TimesheetEntry::TIMESHEET_ID.eq(sheet.id))
        .order_by(TimesheetEntry::DATE)
        .get(&mut *tx)
        .await?;
    let theirs = if other_ids.is_empty() {
        Vec::new()
    } else {
        TimesheetEntry::query()
            .where_(TimesheetEntry::TIMESHEET_ID.is_in(other_ids))
            .where_(TimesheetEntry::DATE.gte(sheet.period_from))
            .where_(TimesheetEntry::DATE.lte(sheet.period_to))
            .get(&mut *tx)
            .await?
    };

    // Every attended day has hours allocated to a cost object.
    for (date, minutes) in &attended {
        if *minutes == 0 {
            continue;
        }
        let allocated = own
            .iter()
            .chain(theirs.iter())
            .any(|e| e.date == *date && e.cost_object().is_some() && e.total_minutes() > 0);
        if !allocated {
            problems.push(english(
                "wl_attendance.submit.not_allocated",
                &[("date", date)],
            ));
        }
    }

    // The hours on a day do not exceed the attended hours.
    let days: BTreeSet<NaiveDate> = own.iter().map(|e| e.date).collect();
    for date in days {
        let mine: i64 = own
            .iter()
            .filter(|e| e.date == date)
            .map(TimesheetEntry::total_minutes)
            .sum();
        let elsewhere: i64 = theirs
            .iter()
            .filter(|e| e.date == date && submitted_ids.contains(&e.timesheet_id))
            .map(TimesheetEntry::total_minutes)
            .sum();
        let present = attended.get(&date).copied().unwrap_or(0);
        let allocated = mine + elsewhere;
        if allocated > present {
            problems.push(english(
                "wl_attendance.submit.over_allocated",
                &[
                    ("date", &date),
                    ("excess", &hours_text(allocated - present)),
                    ("allocated", &hours_text(allocated)),
                    ("attended", &hours_text(present)),
                ],
            ));
        }
    }
    Ok(problems)
}

/// Runs the submit of `sheet_id` by `actor` (a user) or the system, with the
/// lock of each day of the entries held. `Ok(Err(message))` is a refusal of
/// the checks; the timesheet stays `open`.
async fn attempt(
    state: &AppState,
    actor: Option<&AuthUser>,
    sheet_id: i64,
) -> Result<std::result::Result<Timesheet, String>> {
    let found = Timesheet::find_or_404(&state.db, sheet_id).await?;
    if let Some(user) = actor
        && !access::holds_in_context(
            state,
            user.id,
            permissions::TIMESHEETS_SUBMIT,
            found.context().as_ref(),
        )
        .await?
    {
        return Err(Error::Forbidden);
    }
    let days: BTreeSet<NaiveDate> = TimesheetEntry::of_timesheet(&state.db, sheet_id)
        .await?
        .into_iter()
        .map(|e| e.date)
        .collect();
    // In date order, so that two submits cannot wait for each other.
    let mut held = Vec::new();
    for day in days {
        held.push(lock::take(found.owner_employee_id, day).await);
    }
    let mut tx = state.db.begin().await?;
    let sheet = Timesheet::find(&mut tx, sheet_id)
        .await?
        .ok_or(Error::NotFound)?;
    let run = match actor {
        Some(user) => sheet.transition(SUBMIT).by(user),
        None => sheet.transition(SUBMIT_BY_SYSTEM).by_system(),
    };
    let outcome = match run.run_in(&mut tx).await {
        Ok(outcome) => outcome,
        Err(TransitionError::Guard(message)) => return Ok(Err(message)),
        Err(TransitionError::Failed(Error::Status(code, message)))
            if code == StatusCode::UNPROCESSABLE_ENTITY =>
        {
            return Ok(Err(message));
        }
        Err(other) => return Err(other.into_error()),
    };
    tx.commit().await?;
    drop(held);
    outcome.emit(state).await?;
    Ok(Ok(Timesheet::find_or_404(&state.db, sheet_id).await?))
}

/// `actor` submits timesheet `sheet_id` (`timesheets.submit` in the scope of
/// its context).
///
/// # Errors
///
/// `NotFound`; `Forbidden` without the permission; `422` for a timesheet that
/// is not `open`, or with the days and the excess when the checks above
/// refuse.
pub async fn submit(state: &AppState, actor: &AuthUser, sheet_id: i64) -> Result<Timesheet> {
    match attempt(state, Some(actor), sheet_id).await? {
        Ok(sheet) => Ok(sheet),
        Err(message) => Err(refuse(message)),
    }
}

/// What a period-end submit by the system came to.
#[derive(Debug, Clone)]
pub enum SystemSubmit {
    /// It is `submitted`.
    Submitted(Timesheet),
    /// A check refused it: the timesheet stays `open`.
    Refused {
        /// The timesheet, still open.
        timesheet: Timesheet,
        /// The days and the reason.
        message: String,
        /// The task opened for the submitter; none without ticketing or
        /// without a submitter.
        task: Option<TicketRef>,
    },
}

/// The source key of the task of a timesheet whose period-end submit failed.
pub fn task_key(sheet_id: i64) -> String {
    format!("attendance.timesheet_submit:{sheet_id}")
}

/// The system submits timesheet `sheet_id` at period end. A failed check
/// leaves it `open` and gives the submitter a task listing the days.
///
/// # Errors
///
/// `NotFound`; `422` for a timesheet that is not `open`.
pub async fn submit_by_system(state: &AppState, sheet_id: i64) -> Result<SystemSubmit> {
    match attempt(state, None, sheet_id).await? {
        Ok(sheet) => Ok(SystemSubmit::Submitted(sheet)),
        Err(message) => {
            let sheet = Timesheet::find_or_404(&state.db, sheet_id).await?;
            if sheet.status != TimesheetStatus::Open {
                return Err(refuse(message));
            }
            let task = open_task(state, &sheet, &message).await?;
            Ok(SystemSubmit::Refused {
                timesheet: sheet,
                message,
                task,
            })
        }
    }
}

/// Opens (once) the task for the submitter of `sheet`.
async fn open_task(
    state: &AppState,
    sheet: &Timesheet,
    message: &str,
) -> Result<Option<TicketRef>> {
    let creator = tickets::task_creator(state);
    let key = task_key(sheet.id);
    if let Some(found) = creator.find_open(state, &key).await? {
        return Ok(Some(found));
    }
    let submitter = match sheet.created_by {
        Some(user) => Some(user),
        None => workforce(state)
            .staff(state, sheet.owner())
            .await?
            .and_then(|profile| profile.user_id),
    };
    let title = english(
        "wl_attendance.submit.task_title",
        &[("from", &sheet.period_from), ("to", &sheet.period_to)],
    );
    let mut ticket = OpenTicket::new(sheet.subject(), TASK_CATEGORY, &title)
        .description(message)
        .source_key(&key);
    if let Some(user) = submitter {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_of_a_timesheet_has_its_own_key() {
        assert_eq!(task_key(7), "attendance.timesheet_submit:7");
    }
}
