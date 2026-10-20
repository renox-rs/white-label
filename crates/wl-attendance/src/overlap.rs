//! Overlapping attendance (wl-attendance §4 *record*, §5.5 point 4,
//! BL-ATT-01): one employee cannot be present at two places at the same
//! time.
//!
//! - A **self check-in** is refused while the employee is checked in at
//!   another location (they must check out first) or at the same one (a
//!   duplicate); see [`crate::service::check_in_self`].
//! - **Supervisor marking, imports and corrections** that overlap another
//!   record of the same employee, at any location or project, are saved and
//!   flagged `overlap` through the `flag` transition ([`flag_in`]): both
//!   records, or only the new one when the other is `locked`. Records that
//!   are `rejected` do not count, and spans that only touch (one ends at 12:00,
//!   the other starts at 12:00) do not overlap.
//! - The **crew list** shows the supervisor the workers already marked at
//!   another location for the same day ([`clashes_for`]).

use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::calendar::calendar;
use wl_kernel::workforce::EmployeeId;

use crate::flags::{self, FlagType, Pending};
use crate::location::WorkLocation;
use crate::record::{AttendanceRecord, AttendanceStatus, Span};
use crate::texts::english;

/// `at` as `07:00` in Asia/Dubai.
fn clock(calendar: &BusinessCalendar, at: DateTime) -> String {
    calendar.local(at).format("%H:%M").to_string()
}

/// The records of `employee` on `date` and the days around it (a span can
/// cross midnight) that count, oldest first.
async fn around(tx: &mut Transaction, employee: i64, date: NaiveDate) -> Result<Vec<AttendanceRecord>> {
    let days: Vec<NaiveDate> = [
        date.checked_sub_days(Days::new(1)),
        Some(date),
        date.checked_add_days(Days::new(1)),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee))
        .where_(AttendanceRecord::DATE.is_in(days))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .order_by(AttendanceRecord::ID)
        .get(&mut *tx)
        .await?)
}

/// Flags `record` and the records of the same employee it overlaps `overlap`,
/// inside `tx`. A `locked` record cannot change, so only `record` is flagged,
/// with a note that names the locked one.
pub async fn flag_in(
    state: &AppState,
    tx: &mut Transaction,
    record: &AttendanceRecord,
    pending: &mut Pending,
) -> Result {
    if !record.counts() {
        return Ok(());
    }
    let calendar = calendar(state);
    let at = now();
    let mine = record.span(&calendar, at);
    for other in around(tx, record.employee_id, record.date).await? {
        if other.id == record.id || !mine.overlaps(&other.span(&calendar, at)) {
            continue;
        }
        let theirs = other.span(&calendar, at);
        let there = WorkLocation::find(&mut *tx, other.location_id)
            .await?
            .map_or_else(|| format!("#{}", other.location_id), |l| l.name);
        let here = WorkLocation::find(&mut *tx, record.location_id)
            .await?
            .map_or_else(|| format!("#{}", record.location_id), |l| l.name);
        let locked = other.status == AttendanceStatus::Locked;
        let key = if locked {
            "wl_attendance.overlap.note_locked"
        } else {
            "wl_attendance.overlap.note"
        };
        let note_new = english(
            key,
            &[
                ("record", &other.id),
                ("location", &there),
                ("from", &clock(&calendar, theirs.start)),
                ("to", &clock(&calendar, theirs.end)),
            ],
        );
        flags::add_in(tx, record.id, FlagType::Overlap, &note_new, pending).await?;
        if !locked {
            let note_other = english(
                "wl_attendance.overlap.note",
                &[
                    ("record", &record.id),
                    ("location", &here),
                    ("from", &clock(&calendar, mine.start)),
                    ("to", &clock(&calendar, mine.end)),
                ],
            );
            flags::add_in(tx, other.id, FlagType::Overlap, &note_other, pending).await?;
        }
    }
    Ok(())
}

/// A worker of the crew who is already marked at another location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    /// The employee.
    pub employee: EmployeeId,
    /// The record that clashes.
    pub record: i64,
    /// The other location.
    pub location_id: i64,
    /// Its name.
    pub location: String,
    /// When the presence there starts.
    pub from: DateTime,
    /// When it ends; none while the worker is still checked in.
    pub to: Option<DateTime>,
    /// The hours as shown: `07:00–15:00`, or `from 07:00` while open.
    pub hours: String,
}

/// The crew list of `location` on `date`: for each of `employees`, the
/// records they already have at another location that day (not `rejected`).
/// With `hours`, only those that share a minute with them; the supervisor
/// sees the clash before submitting.
pub async fn clashes_for(
    state: &AppState,
    location: i64,
    date: NaiveDate,
    employees: &[EmployeeId],
    hours: Option<(DateTime, DateTime)>,
) -> Result<Vec<Clash>> {
    let calendar = calendar(state);
    let at = now();
    let mut found = Vec::new();
    for employee in employees {
        let records = AttendanceRecord::query()
            .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
            .where_(AttendanceRecord::DATE.eq(date))
            .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
            .order_by(AttendanceRecord::CHECK_IN)
            .get(&state.db)
            .await?;
        for record in records.into_iter().filter(|r| r.location_id != location) {
            let span = record.span(&calendar, at);
            if let Some((start, end)) = hours
                && !span.overlaps(&Span { start, end })
            {
                continue;
            }
            let name = WorkLocation::find(&state.db, record.location_id)
                .await?
                .map_or_else(|| format!("#{}", record.location_id), |l| l.name);
            let shown = match record.check_out {
                Some(out) => format!(
                    "{}\u{2013}{}",
                    clock(&calendar, record.check_in),
                    clock(&calendar, out)
                ),
                None => format!("from {}", clock(&calendar, record.check_in)),
            };
            found.push(Clash {
                employee: *employee,
                record: record.id,
                location_id: record.location_id,
                location: name,
                from: record.check_in,
                to: record.check_out,
                hours: shown,
            });
        }
    }
    Ok(found)
}
