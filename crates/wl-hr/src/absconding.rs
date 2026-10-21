//! `report_absconding` (wl-hr §4 *Employee*, §5.4 *Absconding*, §10, issue
//! #197): after more than N consecutive days of absence with the worker
//! unreachable and a MoHRE absence report filed, HR moves the employee to
//! `absconded`, with the MD's approval (`HRM.ACL.07`, [`crate::exits`]).
//!
//! - **The checks** ([`problem_in`]) read the database only, so they run
//!   at the request, again at the final approval (the kernel's final-check
//!   hook, [`crate::exits`]) and a last time as the first effect of the
//!   transition:
//!   an absence case is open ([`AbsenceCase`]); strictly more than N calendar
//!   days (default 7, Asia/Dubai) have passed since the last day with
//!   attendance or approved leave (the age of the case does not matter: it may
//!   have opened on day 3 or on day 8); and at least two failed contact
//!   attempts were recorded on different days after that day. The report
//!   number and date are checked with the documents ([`crate::exits::missing`]).
//! - **The effects** ([`apply`]), in the transition's transaction: the
//!   absence report is recorded on the employee and the portal and attendance
//!   are switched off with it; the payroll lines in runs not yet `submitted`
//!   are held with *absence report* ([`payroll::hold_open_lines_in`]; a line
//!   in a sent SIF file stays, and is held only if the bank fails it); the
//!   final settlement is drafted, or the one the notice drafted gets the reason
//!   *absconding* and no second is drafted; and the *cancellation* visa case
//!   opens from the *absconding* variant of the step template. `EmployeeAbsconded`
//!   follows the commit, for the apps to end the deployments. What may be
//!   withheld from the worker's dues is `⚠ open: lawyer (L-24)`: the settlement
//!   is drafted as for any exit and nothing is forfeited.

use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::absence::{AbsenceCase, AbsenceReport, ContactAttempt};
use crate::exits::ExitInput;
use crate::leave::{LeaveRequest, LeaveStatus};
use crate::texts::english;
use crate::{payroll, settlement, visa};

pub use crate::config::DEFAULT_ABSENCE_DAYS;

/// Failed contact attempts, on different days, that show the worker cannot be
/// reached.
pub const FAILED_ATTEMPTS: usize = 2;

/// The last day with attendance or approved leave of `employee_id` on `day`:
/// the latest of the open absence cases' last attended day and the end of the
/// approved leave that has started (up to `day`). `None` without an open
/// case.
pub async fn last_present_in(
    tx: &mut Transaction,
    employee_id: i64,
    day: NaiveDate,
) -> Result<Option<NaiveDate>> {
    let cases = AbsenceCase::query()
        .where_(AbsenceCase::EMPLOYEE_ID.eq(employee_id))
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .get(&mut *tx)
        .await?;
    let Some(mut last) = cases.iter().map(|c| c.last_present_on).max() else {
        return Ok(None);
    };
    let leave = LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee_id))
        .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]))
        .get(&mut *tx)
        .await?;
    for leave in leave.iter().filter(|l| l.from_date <= day && l.days() > 0) {
        last = last.max(leave.to_date.min(day));
    }
    Ok(Some(last))
}

/// What stops `report_absconding` for `employee_id` on `day`, in English, or
/// `None` when the database allows it: an absence case is open, more than
/// `absence_days` days passed since the last day present, and two failed
/// contact attempts were recorded on different days after it.
pub async fn problem_in(
    tx: &mut Transaction,
    employee_id: i64,
    day: NaiveDate,
    absence_days: i64,
) -> Result<Option<String>> {
    let Some(last) = last_present_in(&mut *tx, employee_id, day).await? else {
        return Ok(Some(english("wl_hr.absconding.no_case", &[])));
    };
    let days = (day - last).num_days();
    if days <= absence_days {
        let (limit, last_text, passed) =
            (absence_days.to_string(), last.to_string(), days.to_string());
        return Ok(Some(english(
            "wl_hr.absconding.too_early",
            &[("days", &limit), ("last", &last_text), ("passed", &passed)],
        )));
    }
    let mut days_tried: Vec<NaiveDate> = ContactAttempt::query()
        .where_(ContactAttempt::EMPLOYEE_ID.eq(employee_id))
        .get(&mut *tx)
        .await?
        .into_iter()
        .filter(|a| a.outcome.failed() && a.attempted_on > last && a.attempted_on <= day)
        .map(|a| a.attempted_on)
        .collect();
    days_tried.sort_unstable();
    days_tried.dedup();
    if days_tried.len() < FAILED_ATTEMPTS {
        let attempts = FAILED_ATTEMPTS.to_string();
        return Ok(Some(english(
            "wl_hr.absconding.not_unreachable",
            &[("attempts", &attempts)],
        )));
    }
    Ok(None)
}

/// The refusal as an HTTP 422.
fn refusal(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The effect of `report_absconding`, inside the transition's transaction:
/// checks the database once more, then records the report, holds the pay,
/// drafts the settlement and opens the cancellation case.
pub async fn apply(tx: &mut Transaction, employee_id: i64, input: &ExitInput) -> Result {
    let day = today();
    let days = input.absence_days.unwrap_or(DEFAULT_ABSENCE_DAYS);
    if let Some(message) = problem_in(&mut *tx, employee_id, day, days).await? {
        return Err(refusal(message));
    }
    let (Some(number), Some(date)) = (
        input
            .report_number
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty()),
        input.report_date,
    ) else {
        return Err(refusal(english("wl_hr.exits.report_required", &[])));
    };
    AbsenceReport::create(
        &mut *tx,
        AbsenceReport {
            employee_id,
            report_number: number.to_owned(),
            report_date: date,
            report_file_id: input.report_file,
            reported_on: day,
            status: "valid".to_owned(),
            access_disabled_at: Some(now()),
            ..AbsenceReport::default()
        },
    )
    .await?;
    payroll::hold_open_lines_in(&mut *tx, employee_id, payroll::HELD_ABSENCE_REPORT).await?;
    settlement::draft_or_relabel_in(
        &mut *tx,
        employee_id,
        settlement::REASON_ABSCONDING,
        settlement::PAYEE_EMPLOYEE,
    )
    .await?;
    visa::open_absconding_in(&mut *tx, employee_id, number, date).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        assert_eq!(DEFAULT_ABSENCE_DAYS, 7);
        assert_eq!(FAILED_ATTEMPTS, 2);
    }
}
