//! The plan of a batch: the sessions its weekly pattern gives between the
//! start and the end date, around the public holidays (10 §4.2 step 3,
//! `EDU.BAT.01`).
//!
//! Decision: the pattern is the weekdays (1 = Monday ... 7 = Sunday), one start
//! time (Asia/Dubai wall time, `HH:MM`) and one duration, the same for every
//! session of the batch. A holiday is a date of the client's `[calendar]`; only
//! those are skipped, not the weekend (a centre may teach on Saturdays and
//! Sundays), and a skipped date is not added at the end: the batch ends on its
//! end date. The functions here are pure; [`crate::batches`] reads the
//! holidays from the kernel's calendar and checks the tutor and the room.

use std::collections::BTreeSet;

use renox::chrono::{Datelike, NaiveDate, NaiveTime, TimeDelta};
use renox::db::DateTime;
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::BusinessCalendar;

use crate::batch::Batch;

/// One session the plan gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedSession {
    /// 1, 2, 3 ... in date order.
    pub number: i64,
    /// The day (Asia/Dubai).
    pub date: NaiveDate,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
}

/// Why a plan cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanProblem {
    /// The start time is not `HH:MM`.
    StartTime,
    /// The duration is not positive.
    Duration,
    /// No weekday of the pattern is 1 to 7.
    Weekdays,
    /// The end date is before the start date.
    Dates,
}

/// The time `text` (`18:30`), or `None`.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

/// The dates from `start` to `end` (both included) that fall on one of
/// `weekdays` and are not in `holidays`, in date order.
pub fn dates(
    start: NaiveDate,
    end: NaiveDate,
    weekdays: &[i64],
    holidays: &BTreeSet<NaiveDate>,
) -> Vec<NaiveDate> {
    let mut found = Vec::new();
    let mut date = start;
    while date <= end {
        let weekday = i64::from(date.weekday().number_from_monday());
        if weekdays.contains(&weekday) && !holidays.contains(&date) {
            found.push(date);
        }
        date += TimeDelta::days(1);
    }
    found
}

/// The sessions of `batch` around `holidays`.
///
/// # Errors
///
/// A [`PlanProblem`] when the pattern cannot give a session.
pub fn sessions(
    batch: &Batch,
    holidays: &BTreeSet<NaiveDate>,
) -> Result<Vec<PlannedSession>, PlanProblem> {
    let time = parse_time(&batch.start_time).ok_or(PlanProblem::StartTime)?;
    if batch.duration_minutes <= 0 {
        return Err(PlanProblem::Duration);
    }
    if !batch.weekdays.0.iter().any(|day| (1..=7).contains(day)) {
        return Err(PlanProblem::Weekdays);
    }
    if batch.end_date < batch.start_date {
        return Err(PlanProblem::Dates);
    }
    let calendar = BusinessCalendar::default();
    let planned = dates(
        batch.start_date,
        batch.end_date,
        &batch.weekdays.0,
        holidays,
    )
    .into_iter()
    .zip(1..)
    .map(|(date, number)| {
        let starts_at = calendar.from_local(date.and_time(time));
        PlannedSession {
            number,
            date,
            starts_at,
            ends_at: starts_at + TimeDelta::minutes(batch.duration_minutes),
        }
    })
    .collect();
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::db::Json;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn batch() -> Batch {
        Batch {
            start_date: date("2026-11-02"),
            end_date: date("2026-11-15"),
            weekdays: Json(vec![1, 3]),
            start_time: "18:00".to_owned(),
            duration_minutes: 90,
            ..Batch::default()
        }
    }

    #[test]
    fn the_weekly_pattern_gives_a_session_on_each_listed_weekday() {
        let found = dates(
            date("2026-11-02"),
            date("2026-11-15"),
            &[1, 3],
            &BTreeSet::new(),
        );
        // 2 Nov 2026 is a Monday.
        assert_eq!(
            found,
            [
                date("2026-11-02"),
                date("2026-11-04"),
                date("2026-11-09"),
                date("2026-11-11")
            ]
        );
    }

    #[test]
    fn a_public_holiday_is_skipped_and_nothing_is_added_after_the_end() {
        let holidays: BTreeSet<NaiveDate> = [date("2026-11-04")].into_iter().collect();
        let planned = sessions(&batch(), &holidays).expect("a plan");
        let days: Vec<NaiveDate> = planned.iter().map(|s| s.date).collect();
        assert_eq!(
            days,
            [date("2026-11-02"), date("2026-11-09"), date("2026-11-11")]
        );
        let numbers: Vec<i64> = planned.iter().map(|s| s.number).collect();
        assert_eq!(numbers, [1, 2, 3]);
    }

    #[test]
    fn a_session_starts_at_the_dubai_wall_time_and_lasts_the_duration() {
        let planned = sessions(&batch(), &BTreeSet::new()).expect("a plan");
        // 18:00 in Dubai is 14:00 UTC.
        assert_eq!(planned[0].starts_at.to_rfc3339(), "2026-11-02T14:00:00+00:00");
        assert_eq!(planned[0].ends_at.to_rfc3339(), "2026-11-02T15:30:00+00:00");
    }

    #[test]
    fn a_pattern_that_cannot_give_a_session_is_a_problem() {
        let mut bad = batch();
        bad.start_time = "evening".to_owned();
        assert_eq!(sessions(&bad, &BTreeSet::new()), Err(PlanProblem::StartTime));
        let mut bad = batch();
        bad.duration_minutes = 0;
        assert_eq!(sessions(&bad, &BTreeSet::new()), Err(PlanProblem::Duration));
        let mut bad = batch();
        bad.weekdays = Json(vec![0, 8]);
        assert_eq!(sessions(&bad, &BTreeSet::new()), Err(PlanProblem::Weekdays));
        let mut bad = batch();
        bad.end_date = date("2026-10-01");
        assert_eq!(sessions(&bad, &BTreeSet::new()), Err(PlanProblem::Dates));
    }
}
