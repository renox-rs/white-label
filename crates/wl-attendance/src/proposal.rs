//! What a record gives the timesheet (wl-attendance §4 `accept` effects, §5.5
//! point 3, BL-ATT-04). Timesheets come with their own issues; this is the
//! part the checks decide, for them to call.
//!
//! - **Only `accept` proposes hours.** While a record is `recorded`,
//!   `flagged` or `rejected` it proposes none ([`proposed`]). A record that is
//!   `flagged` also holds the timesheet's `submit` ([`submit_blocker`]), which
//!   refuses with a message naming the record.
//! - **A `permit_invalid` record proposes its hours non-billable**, and they
//!   count for payroll all the same (the default of `⚠ open: lawyer, L-28`:
//!   paid, not billed until HR reviews). The submitter may set the entry
//!   billable before `submit`; that change is the entry's history, not
//!   this module's.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::calendar::calendar;
use wl_kernel::workforce::EmployeeId;

use crate::flags::{AttendanceFlag, FlagType};
use crate::location::WorkLocation;
use crate::overtime::worked_minutes;
use crate::record::{AttendanceRecord, AttendanceStatus};
use crate::texts::english;

/// The hours an accepted record proposes on the open timesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proposal {
    /// The minutes worked, less the breaks taken.
    pub minutes: i64,
    /// Whether the hours are proposed billable: not when the record carries
    /// `permit_invalid`.
    pub billable: bool,
}

/// What `record`, with its `flags`, proposes: nothing unless it is
/// `accepted` (a `flagged` record proposes none until the reviewer accepts
/// it); non-billable hours for a `permit_invalid` record.
pub fn proposed(
    calendar: &BusinessCalendar,
    record: &AttendanceRecord,
    flags: &[AttendanceFlag],
) -> Option<Proposal> {
    if record.status != AttendanceStatus::Accepted {
        return None;
    }
    Some(Proposal {
        minutes: worked_minutes(calendar, record),
        billable: !flags.iter().any(|f| f.flag_type == FlagType::PermitInvalid),
    })
}

/// [`proposed`] for the record `record_id` as stored.
///
/// # Errors
///
/// `NotFound` for an unknown record; the database's errors.
pub async fn proposal_of(state: &AppState, record_id: i64) -> Result<Option<Proposal>> {
    let record = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    let flags = AttendanceFlag::of_record(&state.db, record_id).await?;
    Ok(proposed(&calendar(state), &record, &flags))
}

/// The first record of `employee` from `from` to `to` that is `flagged`: the
/// timesheet's `submit` is refused while there is one.
///
/// # Errors
///
/// The database's errors.
pub async fn first_flagged(
    state: &AppState,
    employee: EmployeeId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Option<AttendanceRecord>> {
    Ok(AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::STATUS.eq(AttendanceStatus::Flagged))
        .order_by(AttendanceRecord::DATE)
        .order_by(AttendanceRecord::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .find(|record| from <= record.date && record.date <= to))
}

/// The message with which the timesheet's `submit` is refused when the
/// attendance of `employee` from `from` to `to` has a `flagged` record; it
/// names the record's day and location. `None` when none is flagged.
///
/// # Errors
///
/// The database's errors.
pub async fn submit_blocker(
    state: &AppState,
    employee: EmployeeId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Option<String>> {
    let Some(record) = first_flagged(state, employee, from, to).await? else {
        return Ok(None);
    };
    let location = WorkLocation::find(&state.db, record.location_id)
        .await?
        .map_or_else(|| format!("#{}", record.location_id), |l| l.name);
    let flags: Vec<String> = AttendanceFlag::of_record(&state.db, record.id)
        .await?
        .into_iter()
        .map(|f| f.flag_type.as_str().to_owned())
        .collect();
    Ok(Some(english(
        "wl_attendance.submit.flagged_record",
        &[
            ("record", &record.id),
            ("date", &record.date),
            ("location", &location),
            ("flags", &flags.join(", ")),
        ],
    )))
}
