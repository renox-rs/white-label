//! The midday break on marked hours (`HRM.RUL.13`, wl-attendance §5.1 step 2,
//! §5.5 point 2, §7, BL-ATT-03): outdoor work is prohibited from 12:30 to
//! 15:00 from 15 June to 15 September. Crews marked as a block by a supervisor
//! (07:00 to 17:00) slip past a check of the check-in time, so the worked
//! span minus the breaks taken is checked too: when any of it falls inside the
//! window, the record is flagged `midday_break` and the supervisor and the HSE
//! officer are told. It flags, it does not block (some work is exempt).
//!
//! - **Pre-fill.** In the season, at an outdoor location, the break of the
//!   crew marking is pre-filled with the window ([`prefill`]); the supervisor
//!   keeps or removes it. Indoors, and out of season, nothing is pre-filled.
//! - **The window is read per scope** ([`window_for`]): the rows of
//!   [`MiddayWindow`] for the scope of the location and the year of the
//!   date; without any, the rows with no scope; without any, the default of
//!   [`AttendanceConfig`] (inclusive month and day bounds). A row set for the
//!   year replaces the default for that year, so a window announced for July
//!   and August does not also apply in June.
//! - **Hours only.** A record with no check-out has no worked span yet and is
//!   not checked; the manual timesheet entries with from and to times are the
//!   timesheet issue's. An entry with hours only has no span and is not
//!   checked.
//! - **Not held for review.** `midday_break` is not one of the three flags
//!   that hold a record (`HRM.ACL.09`); any `attendance.review` holder may
//!   accept it. A record already flagged `midday_break` is not flagged again.

use renox::chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::BusinessCalendar;
use wl_kernel::Subject;
use wl_kernel::calendar::calendar;

use crate::config::config_of;
use crate::flags::{self, FlagType, Pending};
use crate::location::WorkLocation;
use crate::record::{AttendanceRecord, BreakSpan, minutes_between};
use crate::texts::english;

/// The midday break of a scope for one season.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_attendance_midday_windows",
    index(scope_type, scope_id, season_from)
)]
pub struct MiddayWindow {
    /// The primary key.
    pub id: i64,
    /// The type of the scope's `Subject`; none for every scope.
    pub scope_type: Option<String>,
    /// The id of the scope's `Subject`.
    pub scope_id: Option<i64>,
    /// The first day of the season.
    pub season_from: NaiveDate,
    /// The last day of the season, included.
    pub season_to: NaiveDate,
    /// When the break starts, in minutes after midnight (Asia/Dubai).
    pub starts_minute: i64,
    /// When the break ends, in minutes after midnight.
    pub ends_minute: i64,
    /// When the row was created.
    pub created_at: Option<renox::db::DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<renox::db::DateTime>,
}

impl MiddayWindow {
    /// The window from `starts` to `ends` (hours and minutes) from `from` to
    /// `to`, for `scope` (none: every scope).
    ///
    /// # Panics
    ///
    /// When an hour or minute is out of range.
    pub fn new(
        scope: Option<&Subject>,
        from: NaiveDate,
        to: NaiveDate,
        starts: (u32, u32),
        ends: (u32, u32),
    ) -> Self {
        MiddayWindow {
            scope_type: scope.map(|s| s.subject_type.clone()),
            scope_id: scope.map(|s| s.subject_id),
            season_from: from,
            season_to: to,
            starts_minute: i64::from(starts.0 * 60 + starts.1),
            ends_minute: i64::from(ends.0 * 60 + ends.1),
            ..MiddayWindow::default()
        }
    }

    /// Whether the season covers `date`.
    pub fn covers(&self, date: NaiveDate) -> bool {
        self.season_from <= date && date <= self.season_to
    }
}

/// The break window of a day: from `starts` to `ends`, Asia/Dubai.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// When the break starts.
    pub starts: NaiveTime,
    /// When the break ends.
    pub ends: NaiveTime,
}

impl Window {
    /// The window as the break to pre-fill.
    pub fn as_break(&self) -> BreakSpan {
        BreakSpan {
            from: self.starts,
            to: self.ends,
        }
    }
}

/// `minutes` after midnight as a time of day.
fn time_of(minutes: i64) -> NaiveTime {
    let minutes = u32::try_from(minutes.clamp(0, 24 * 60 - 1)).unwrap_or(0);
    NaiveTime::from_hms_opt(minutes / 60, minutes % 60, 0).unwrap_or(NaiveTime::MIN)
}

/// The window for `date`, given the rows of one scope (or of none) and the
/// year they are for; `None` when there are no rows of that year.
fn from_rows(rows: &[MiddayWindow], date: NaiveDate) -> Option<Option<Window>> {
    let of_year: Vec<&MiddayWindow> = rows
        .iter()
        .filter(|row| row.season_from.year() == date.year())
        .collect();
    if of_year.is_empty() {
        return None;
    }
    Some(
        of_year
            .into_iter()
            .filter(|row| row.covers(date))
            .max_by_key(|row| row.id)
            .map(|row| Window {
                starts: time_of(row.starts_minute),
                ends: time_of(row.ends_minute),
            }),
    )
}

/// The window of the default season (month and day bounds, inclusive).
fn default_window(config: &crate::config::AttendanceConfig, date: NaiveDate) -> Option<Window> {
    let today = (date.month(), date.day());
    let midday = config.midday;
    (midday.from <= today && today <= midday.to).then_some(Window {
        starts: midday.starts,
        ends: midday.ends,
    })
}

/// The midday break window at `location` on `date`: none at an indoor
/// location or out of the season.
pub async fn window_for(
    state: &AppState,
    tx: &mut Transaction,
    location: &WorkLocation,
    date: NaiveDate,
) -> Result<Option<Window>> {
    if !location.outdoor {
        return Ok(None);
    }
    if let Some(scope) = location.scope() {
        let rows = MiddayWindow::query()
            .where_(MiddayWindow::SCOPE_TYPE.eq(scope.subject_type.clone()))
            .where_(MiddayWindow::SCOPE_ID.eq(scope.subject_id))
            .get(&mut *tx)
            .await?;
        if let Some(found) = from_rows(&rows, date) {
            return Ok(found);
        }
    }
    let everyone: Vec<MiddayWindow> = MiddayWindow::query()
        .get(&mut *tx)
        .await?
        .into_iter()
        .filter(|row| row.scope_type.is_none())
        .collect();
    if let Some(found) = from_rows(&everyone, date) {
        return Ok(found);
    }
    Ok(default_window(&config_of(state), date))
}

/// The break the crew marking page of `location` pre-fills for `date`: the
/// window in the season at an outdoor location, else none. The supervisor
/// keeps or removes it.
///
/// # Errors
///
/// `NotFound` for an unknown location; the database's errors.
pub async fn prefill(
    state: &AppState,
    location: i64,
    date: NaiveDate,
) -> Result<Option<BreakSpan>> {
    let location = WorkLocation::find(&state.db, location)
        .await?
        .ok_or(Error::NotFound)?;
    let mut tx = state.db.begin().await?;
    let window = window_for(state, &mut tx, &location, date).await?;
    Ok(window.map(|w| w.as_break()))
}

/// The minutes of the worked span (check-in to check-out, less the breaks
/// taken) of `record` that fall inside `window`.
pub fn worked_inside(
    calendar: &BusinessCalendar,
    record: &AttendanceRecord,
    window: &Window,
) -> i64 {
    let Some(out) = record.check_out else {
        return 0;
    };
    let clip = |from: DateTime, to: DateTime, within: (DateTime, DateTime)| {
        minutes_between(from.max(within.0), to.min(within.1))
    };
    let start = calendar.from_local(record.date.and_time(window.starts));
    let end = calendar.from_local(record.date.and_time(window.ends));
    let span = (record.check_in, out);
    let inside = clip(start, end, span);
    let broken: i64 = record
        .breaks
        .iter()
        .map(|taken| {
            let from = calendar.from_local(record.date.and_time(taken.from));
            let to = calendar.from_local(record.date.and_time(taken.to));
            let in_span = (from.max(span.0), to.min(span.1));
            clip(in_span.0, in_span.1.max(in_span.0), (start, end))
        })
        .sum();
    (inside - broken).max(0)
}

/// `HRM.RUL.13`, midday break: flags `record` `midday_break` when it is at an
/// outdoor location in the season and its worked span minus the breaks
/// crosses the window.
pub async fn check_in(
    state: &AppState,
    tx: &mut Transaction,
    record: &AttendanceRecord,
    location: &WorkLocation,
    pending: &mut Pending,
) -> Result {
    if !record.counts() || record.check_out.is_none() {
        return Ok(());
    }
    let Some(window) = window_for(state, tx, location, record.date).await? else {
        return Ok(());
    };
    let calendar = calendar(state);
    let minutes = worked_inside(&calendar, record, &window);
    if minutes == 0 {
        return Ok(());
    }
    let note = english(
        "wl_attendance.midday.note",
        &[
            ("minutes", &minutes),
            (
                "from",
                &format!("{:02}:{:02}", window.starts.hour(), window.starts.minute()),
            ),
            (
                "to",
                &format!("{:02}:{:02}", window.ends.hour(), window.ends.minute()),
            ),
        ],
    );
    flags::add_in(tx, record.id, FlagType::MiddayBreak, &note, pending).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AttendanceConfig;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn window() -> Window {
        Window {
            starts: NaiveTime::from_hms_opt(12, 30, 0).unwrap(),
            ends: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        }
    }

    fn marked(
        calendar: &BusinessCalendar,
        from: (u32, u32),
        to: (u32, u32),
        breaks: Vec<BreakSpan>,
    ) -> AttendanceRecord {
        let date = day(7, 10);
        AttendanceRecord {
            date,
            check_in: calendar.from_local(date.and_hms_opt(from.0, from.1, 0).unwrap()),
            check_out: Some(calendar.from_local(date.and_hms_opt(to.0, to.1, 0).unwrap())),
            breaks: renox::db::Json(breaks),
            ..AttendanceRecord::default()
        }
    }

    #[test]
    fn the_default_season_runs_from_15_june_to_15_september_inclusive() {
        let config = AttendanceConfig::default();
        assert_eq!(default_window(&config, day(6, 14)), None);
        assert_eq!(default_window(&config, day(6, 15)), Some(window()));
        assert_eq!(default_window(&config, day(9, 15)), Some(window()));
        assert_eq!(default_window(&config, day(9, 16)), None);
        assert_eq!(default_window(&config, day(10, 1)), None);
    }

    #[test]
    fn a_block_marking_with_the_break_kept_crosses_nothing_and_without_it_crosses_the_window() {
        let calendar = BusinessCalendar::default();
        let kept = marked(
            &calendar,
            (7, 0),
            (17, 0),
            vec![BreakSpan::hm((12, 30), (15, 0))],
        );
        assert_eq!(worked_inside(&calendar, &kept, &window()), 0);
        let removed = marked(&calendar, (7, 0), (17, 0), Vec::new());
        assert_eq!(worked_inside(&calendar, &removed, &window()), 150);
        // A partial break leaves 14:00 to 15:00 worked inside the window.
        let partial = marked(
            &calendar,
            (7, 0),
            (17, 0),
            vec![BreakSpan::hm((12, 30), (14, 0))],
        );
        assert_eq!(worked_inside(&calendar, &partial, &window()), 60);
        // Work that stops at 12:30 or starts at 15:00 does not cross it.
        let morning = marked(&calendar, (7, 0), (12, 30), Vec::new());
        assert_eq!(worked_inside(&calendar, &morning, &window()), 0);
        let evening = marked(&calendar, (15, 0), (19, 0), Vec::new());
        assert_eq!(worked_inside(&calendar, &evening, &window()), 0);
    }

    #[test]
    fn a_row_for_the_year_replaces_the_default_and_the_scope_wins() {
        let everyone = MiddayWindow {
            id: 1,
            ..MiddayWindow::new(None, day(7, 1), day(8, 31), (12, 0), (15, 30))
        };
        let date = day(7, 10);
        let found = from_rows(std::slice::from_ref(&everyone), date)
            .unwrap()
            .unwrap();
        assert_eq!(found.starts, NaiveTime::from_hms_opt(12, 0, 0).unwrap());
        // The year has a row, and June is outside it: no window, not the default.
        assert_eq!(
            from_rows(std::slice::from_ref(&everyone), day(6, 20)),
            Some(None)
        );
        // No row for the year: the caller falls back.
        let last_year = MiddayWindow {
            season_from: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            season_to: NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            ..everyone
        };
        assert_eq!(from_rows(&[last_year], date), None);
    }
}
