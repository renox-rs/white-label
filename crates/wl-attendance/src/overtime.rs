//! The overtime cap of `HRM.RUL.13` (wl-attendance §5.5 point 1, §6
//! *Automations*, §7, BL-ATT-02): the UAE labour law allows at most 2 hours of
//! overtime a day and 144 hours in any three weeks. When the hours of an
//! employee go above that, the day's records are flagged `overtime_cap` for
//! the reviewer. The flag never blocks and never trims hours: the law has
//! exceptions (`⚠ open: lawyer, L-27`), and the reviewer writes the one that
//! applies in the reason; no exception logic is modelled.
//!
//! - **The day.** The hours of the employee on the date, over every location,
//!   the attendance records that are not `rejected` and the timesheet entries
//!   ([`EntryHours`]), against the calendar's normal hours of that date
//!   (0 on a rest day or a public holiday, 6 in Ramadan with the default
//!   cut) plus the daily cap (2 h). Overtime of a day is every hour above its
//!   normal hours, as `OvertimePolicy`'s default splits it.
//! - **The window.** The overtime of the 21 days that end on the date, the
//!   date included, against 144 h. Exactly 144 h is not above the cap.
//! - **Which records.** Every record of the employee that day that is not
//!   `locked` or `rejected`, so that the reviewer sees the whole day; a flag
//!   type is held at most once per record. When every record of the day is
//!   `locked`, nothing can be flagged and the supervisor is told instead
//!   ([`on_entry_saved`]).
//! - **Hours.** A record with no check-out has no hours yet; the breaks taken
//!   are taken off. A record belongs to the day of its check-in, also when it
//!   runs past midnight.
//! - **Timesheet entries.** Timesheets are another issue. [`EntryHours`] is
//!   what they provide: the minutes of manual or generated entries (not the
//!   ones proposed from attendance, which would count twice) of an employee on
//!   a date. When an entry is saved the timesheets call [`on_entry_saved`].
//!   Without a provider there are none.
//! - **Settings.** The caps come from [`AttendanceConfig`]; nothing is
//!   hard-coded.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::{Days, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::calendar::calendar;
use wl_kernel::workforce::EmployeeId;

use crate::config::{AttendanceConfig, config_of};
use crate::flags::{self, FlagType, Pending};
use crate::lock;
use crate::notices;
use crate::record::{AttendanceRecord, AttendanceStatus, minutes_between};
use crate::texts::english;

/// The future [`EntryHours`] returns.
pub type HoursFuture<'a> = Pin<Box<dyn Future<Output = Result<i64>> + Send + 'a>>;

/// The hours of timesheet entries that do not come from attendance (manual
/// or generated ones), which `HRM.RUL.13` adds to the attendance of the day.
/// The timesheets of this module provide it; without them there are none.
pub trait EntryHours: Send + Sync + 'static {
    /// The minutes entered for `employee` on `date`. The default is none.
    fn minutes_on<'a>(
        &'a self,
        _state: &'a AppState,
        _employee: EmployeeId,
        _date: NaiveDate,
    ) -> HoursFuture<'a> {
        Box::pin(async { Ok(0) })
    }
}

/// No timesheet entries.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEntryHours;

impl EntryHours for NoEntryHours {}

/// Makes `hours` the app's [`EntryHours`].
pub fn provide_entry_hours(app: &mut Registry, hours: impl EntryHours) {
    let hours: Arc<dyn EntryHours> = Arc::new(hours);
    app.provide(hours);
}

/// The app's [`EntryHours`], or [`NoEntryHours`] when none is provided.
pub fn entry_hours(state: &AppState) -> Arc<dyn EntryHours> {
    match state.provided::<Arc<dyn EntryHours>>() {
        Some(hours) => Arc::clone(&hours),
        None => Arc::new(NoEntryHours),
    }
}

/// The minutes worked in `record`: check-in to check-out less the breaks
/// taken; none while there is no check-out.
pub fn worked_minutes(calendar: &BusinessCalendar, record: &AttendanceRecord) -> i64 {
    let Some(out) = record.check_out else {
        return 0;
    };
    let total = minutes_between(record.check_in, out);
    let breaks: i64 = record
        .breaks
        .iter()
        .map(|taken| {
            let from = calendar.from_local(record.date.and_time(taken.from));
            let to = calendar.from_local(record.date.and_time(taken.to));
            minutes_between(from.max(record.check_in), to.min(out))
        })
        .sum();
    (total - breaks).max(0)
}

/// Why a day is over the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The hours of the day are above the normal hours plus the daily cap.
    Day {
        /// The minutes worked that day.
        total: i64,
        /// The normal minutes of that day.
        normal: i64,
    },
    /// The overtime of the window is above the window cap.
    Window {
        /// The minutes of overtime in the window.
        overtime: i64,
    },
}

/// The overtime of a day: the minutes above its normal minutes.
pub fn overtime_of(total: i64, normal: i64) -> i64 {
    (total - normal).max(0)
}

/// The days of the window that ends on `date`, oldest first.
pub fn window_days(config: &AttendanceConfig, date: NaiveDate) -> Vec<NaiveDate> {
    let back = u64::try_from(config.overtime_window_days.max(1) - 1).unwrap_or(0);
    let mut days = Vec::new();
    let mut day = date.checked_sub_days(Days::new(back)).unwrap_or(date);
    while day <= date {
        days.push(day);
        match day.checked_add_days(Days::new(1)) {
            Some(next) => day = next,
            None => break,
        }
    }
    days
}

/// Whether the hours of `date` break the cap, given the minutes worked on each
/// day of the window. The daily cap is tried first.
pub fn verdict(
    config: &AttendanceConfig,
    calendar: &BusinessCalendar,
    totals: &BTreeMap<NaiveDate, i64>,
    date: NaiveDate,
) -> Option<Reason> {
    let total = totals.get(&date).copied().unwrap_or(0);
    let normal = calendar.normal_minutes_on(date);
    if total > normal + config.overtime_day_minutes {
        return Some(Reason::Day { total, normal });
    }
    let overtime: i64 = window_days(config, date)
        .into_iter()
        .map(|day| {
            overtime_of(
                totals.get(&day).copied().unwrap_or(0),
                calendar.normal_minutes_on(day),
            )
        })
        .sum();
    (overtime > config.overtime_window_minutes).then_some(Reason::Window { overtime })
}

/// `minutes` as hours with one decimal: `10.5`.
fn hours_text(minutes: i64) -> String {
    format!("{:.1}", (minutes as f64) / 60.0)
}

/// The note of the flag.
fn note_of(config: &AttendanceConfig, reason: Reason, date: NaiveDate) -> String {
    match reason {
        Reason::Day { total, normal } => english(
            "wl_attendance.overtime.note_day",
            &[
                ("total", &hours_text(total)),
                ("normal", &hours_text(normal)),
                ("cap", &hours_text(config.overtime_day_minutes)),
            ],
        ),
        Reason::Window { overtime } => english(
            "wl_attendance.overtime.note_window",
            &[
                ("days", &config.overtime_window_days),
                ("date", &date),
                ("overtime", &hours_text(overtime)),
                ("cap", &hours_text(config.overtime_window_minutes)),
            ],
        ),
    }
}

/// The minutes worked on each day of `days` by `employee`: the attendance
/// that is not `rejected` and the timesheet entries.
async fn totals_in(
    state: &AppState,
    tx: &mut Transaction,
    employee: EmployeeId,
    days: &[NaiveDate],
    calendar: &BusinessCalendar,
) -> Result<BTreeMap<NaiveDate, i64>> {
    let mut totals: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let records = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.is_in(days.to_vec()))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .get(&mut *tx)
        .await?;
    for record in &records {
        *totals.entry(record.date).or_default() += worked_minutes(calendar, record);
    }
    let entries = entry_hours(state);
    for day in days {
        let minutes = entries.minutes_on(state, employee, *day).await?;
        if minutes > 0 {
            *totals.entry(*day).or_default() += minutes;
        }
    }
    Ok(totals)
}

/// What the check of a day found.
#[derive(Debug, Clone)]
pub enum DayCheck {
    /// The day is within the cap.
    Within,
    /// The day is over the cap.
    Over {
        /// Why.
        reason: Reason,
        /// Whether a record could take the flag: the day has a record that
        /// is not `locked` or `rejected`.
        flaggable: bool,
        /// The `locked` records of the day.
        locked: Vec<AttendanceRecord>,
        /// The note of the flag.
        note: String,
    },
}

/// Checks the hours of `employee` on `date` inside `tx` and flags the day's
/// records `overtime_cap` when they are over the cap.
pub async fn check_day_in(
    state: &AppState,
    tx: &mut Transaction,
    employee: EmployeeId,
    date: NaiveDate,
    pending: &mut Pending,
) -> Result<DayCheck> {
    let config = config_of(state);
    let calendar = calendar(state);
    let days = window_days(&config, date);
    let totals = totals_in(state, tx, employee, &days, &calendar).await?;
    let Some(reason) = verdict(&config, &calendar, &totals, date) else {
        return Ok(DayCheck::Within);
    };
    let note = note_of(&config, reason, date);
    let records = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.eq(date))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .order_by(AttendanceRecord::ID)
        .get(&mut *tx)
        .await?;
    let mut flaggable = false;
    let mut locked = Vec::new();
    for record in records {
        if record.status == AttendanceStatus::Locked {
            locked.push(record);
            continue;
        }
        flaggable = true;
        flags::add_in(tx, record.id, FlagType::OvertimeCap, &note, pending).await?;
    }
    Ok(DayCheck::Over {
        reason,
        flaggable,
        locked,
        note,
    })
}

/// `HRM.RUL.13` for a record that was saved or corrected: the day of the
/// record is checked when the record has hours.
pub async fn check_in(
    state: &AppState,
    tx: &mut Transaction,
    record: &AttendanceRecord,
    pending: &mut Pending,
) -> Result {
    if !record.counts() || record.check_out.is_none() {
        return Ok(());
    }
    check_day_in(state, tx, record.employee(), record.date, pending)
        .await
        .map(|_| ())
}

/// `HRM.RUL.13` for a timesheet entry that was saved: the day of `employee` is
/// checked again, with the entries ([`EntryHours`]) already stored. The
/// day's open records are flagged; when all of them are `locked` the
/// supervisors of the records are told without a flag.
///
/// # Errors
///
/// The database's errors.
pub async fn on_entry_saved(
    state: &AppState,
    employee: EmployeeId,
    date: NaiveDate,
) -> Result<DayCheck> {
    let _lock = lock::take(employee.0, date).await;
    let mut tx = state.db.begin().await?;
    let mut pending = Pending::default();
    let found = check_day_in(state, &mut tx, employee, date, &mut pending).await?;
    tx.commit().await?;
    pending.finish(state).await?;
    if let DayCheck::Over {
        flaggable: false,
        locked,
        note,
        ..
    } = &found
        && !locked.is_empty()
    {
        notices::over_cap_unflagged(state, locked, note).await?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::BreakSpan;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn at(calendar: &BusinessCalendar, date: NaiveDate, h: u32, m: u32) -> renox::db::DateTime {
        calendar.from_local(date.and_hms_opt(h, m, 0).unwrap())
    }

    #[test]
    fn the_hours_of_a_record_are_in_to_out_less_the_breaks() {
        let calendar = BusinessCalendar::default();
        let date = day(11, 3);
        let mut record = AttendanceRecord {
            date,
            check_in: at(&calendar, date, 7, 0),
            check_out: Some(at(&calendar, date, 17, 0)),
            ..AttendanceRecord::default()
        };
        assert_eq!(worked_minutes(&calendar, &record), 10 * 60);
        record.breaks = renox::db::Json(vec![BreakSpan::hm((12, 30), (15, 0))]);
        assert_eq!(worked_minutes(&calendar, &record), 450);
        // A break outside the span takes nothing off.
        record.breaks = renox::db::Json(vec![BreakSpan::hm((18, 0), (19, 0))]);
        assert_eq!(worked_minutes(&calendar, &record), 10 * 60);
        record.check_out = None;
        assert_eq!(worked_minutes(&calendar, &record), 0);
    }

    #[test]
    fn a_day_is_over_the_cap_above_the_normal_hours_plus_two() {
        let config = AttendanceConfig::default();
        let calendar = BusinessCalendar::default();
        let tuesday = day(11, 3);
        let hours = |h: i64| BTreeMap::from([(tuesday, h * 60)]);
        assert_eq!(verdict(&config, &calendar, &hours(10), tuesday), None);
        assert_eq!(
            verdict(&config, &calendar, &BTreeMap::from([(tuesday, 630)]), tuesday),
            Some(Reason::Day {
                total: 630,
                normal: 480
            })
        );
        // On a rest day every hour is overtime and the cap is 2 h.
        let saturday = day(11, 7);
        assert_eq!(
            verdict(&config, &calendar, &BTreeMap::from([(saturday, 120)]), saturday),
            None
        );
        assert!(
            verdict(&config, &calendar, &BTreeMap::from([(saturday, 121)]), saturday).is_some()
        );
    }

    #[test]
    fn ramadan_makes_the_normal_hours_two_hours_shorter() {
        let config = AttendanceConfig::default();
        let calendar =
            BusinessCalendar::default().with_reduced_hours(day(11, 9), day(11, 13), 2);
        let tuesday = day(11, 10);
        let eight_and_a_half = BTreeMap::from([(tuesday, 510)]);
        assert!(verdict(&config, &calendar, &eight_and_a_half, tuesday).is_some());
        let eight = BTreeMap::from([(tuesday, 480)]);
        assert_eq!(verdict(&config, &calendar, &eight, tuesday), None);
    }

    #[test]
    fn the_window_is_the_21_days_that_end_on_the_date() {
        let config = AttendanceConfig::default();
        let days = window_days(&config, day(11, 30));
        assert_eq!(days.len(), 21);
        assert_eq!(days[0], day(11, 10));
        assert_eq!(days[20], day(11, 30));
    }

    #[test]
    fn overtime_above_144_hours_in_the_window_is_over_the_cap_and_144_is_not() {
        let config = AttendanceConfig::default();
        let calendar = BusinessCalendar::default();
        let monday = day(11, 30);
        // 20 days at 7 h of overtime and the 30th at 2 h: 142 h, plus `bump`
        // hours on the 12th. Rest days have no normal hours, so every hour
        // there is overtime.
        let totals = |bump: i64| {
            window_days(&config, monday)
                .into_iter()
                .map(|day| {
                    let normal = calendar.normal_minutes_on(day);
                    let extra = match day {
                        d if d == monday => 120,
                        d if d == self::day(11, 12) => 420 + bump * 60,
                        _ => 420,
                    };
                    (day, normal + extra)
                })
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(verdict(&config, &calendar, &totals(0), monday), None);
        // Exactly 144 h is not above the cap.
        assert_eq!(verdict(&config, &calendar, &totals(2), monday), None);
        assert_eq!(
            verdict(&config, &calendar, &totals(3), monday),
            Some(Reason::Window { overtime: 145 * 60 })
        );
    }

    #[test]
    fn the_notes_give_the_hours() {
        let config = AttendanceConfig::default();
        let note = note_of(
            &config,
            Reason::Day {
                total: 630,
                normal: 480,
            },
            day(11, 3),
        );
        assert!(note.contains("10.5") && note.contains("8.0") && note.contains("2.0"), "{note}");
        let window = note_of(&config, Reason::Window { overtime: 8700 }, day(11, 30));
        assert!(window.contains("145.0") && window.contains("144.0"), "{window}");
    }
}
