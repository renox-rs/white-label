//! The business calendar: working days and hours in Asia/Dubai (wl-kernel §6
//! `BusinessCalendar`).
//!
//! Asia/Dubai is UTC+04:00 all year (no daylight saving). The weekend is
//! Saturday and Sunday, a working day runs 09:00–18:00, and the client's
//! public holidays come from `client.toml`:
//!
//! ```toml
//! [calendar]
//! holidays = ["2026-12-02", "2026-12-03"]
//! ```
//!
//! Normal hours: a working day has 8 normal hours, a rest day or a public
//! holiday has none, and a reduced-hours period (Ramadan: two hours less,
//! ER Art. 15(2)) takes hours off the days it covers:
//!
//! ```toml
//! [calendar]
//! normal_hours = 8
//!
//! [[calendar.reduced_hours]]
//! from = "2027-02-18"
//! to = "2027-03-19"
//! hours_less = 2
//! ```
//!
//! [`BusinessCalendar::normal_minutes_on`] answers for a date. Modules that
//! check the hours worked against the normal hours (wl-attendance
//! `HRM.RUL.13`) read it.
//!
//! The kernel provides the client's calendar; modules read it with
//! [`calendar`]. SLA due times are [`BusinessCalendar::add_working`] from the
//! time of the action, and paused time is
//! [`BusinessCalendar::working_minutes_between`]. Scopes and a holiday table
//! come later behind the same type.

use std::collections::BTreeSet;
use std::sync::Arc;

use renox::chrono::{
    DateTime as ChronoDateTime, Datelike, Days, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime,
    TimeDelta, Weekday,
};
use renox::db::DateTime;
use renox::prelude::*;

use crate::documents::DUBAI_OFFSET_SECONDS;

/// The daily normal hours of a working day when the client sets none: 8
/// (Art. 17 of the labour law).
pub const DEFAULT_NORMAL_MINUTES: i64 = 8 * 60;

/// A period in which the normal hours are shorter, e.g. Ramadan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReducedHours {
    /// The first day of the period.
    pub from: NaiveDate,
    /// The last day of the period, included.
    pub to: NaiveDate,
    /// The minutes taken off the normal hours of each day in it.
    pub minutes_less: i64,
}

/// The working days, hours and holidays of one client, in Asia/Dubai.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessCalendar {
    holidays: BTreeSet<NaiveDate>,
    normal_minutes: i64,
    reduced: Vec<ReducedHours>,
}

impl Default for BusinessCalendar {
    fn default() -> Self {
        BusinessCalendar::new([])
    }
}

impl BusinessCalendar {
    /// When a working day opens: 09:00.
    pub const OPENS: NaiveTime = NaiveTime::from_hms_opt(9, 0, 0).expect("09:00");
    /// When a working day closes: 18:00.
    pub const CLOSES: NaiveTime = NaiveTime::from_hms_opt(18, 0, 0).expect("18:00");

    /// A calendar with these public holidays.
    pub fn new(holidays: impl IntoIterator<Item = NaiveDate>) -> Self {
        BusinessCalendar {
            holidays: holidays.into_iter().collect(),
            normal_minutes: DEFAULT_NORMAL_MINUTES,
            reduced: Vec::new(),
        }
    }

    /// With `hours` normal hours on a working day (8 by default).
    pub fn with_normal_hours(mut self, hours: u32) -> Self {
        self.normal_minutes = i64::from(hours) * 60;
        self
    }

    /// With the normal hours `hours_less` shorter from `from` to `to`
    /// (included), e.g. in Ramadan. Where periods meet, the longest cut
    /// counts.
    pub fn with_reduced_hours(mut self, from: NaiveDate, to: NaiveDate, hours_less: u32) -> Self {
        self.reduced.push(ReducedHours {
            from,
            to,
            minutes_less: i64::from(hours_less) * 60,
        });
        self
    }

    /// The reduced-hours periods, in the order they were added.
    pub fn reduced_hours(&self) -> &[ReducedHours] {
        &self.reduced
    }

    /// The normal minutes of `date`: none on a rest day or a public holiday,
    /// else the daily normal hours less the cut of a reduced-hours period
    /// that covers the day (never below zero).
    pub fn normal_minutes_on(&self, date: NaiveDate) -> i64 {
        if !self.is_working_day(date) {
            return 0;
        }
        let less = self
            .reduced
            .iter()
            .filter(|period| period.from <= date && date <= period.to)
            .map(|period| period.minutes_less)
            .max()
            .unwrap_or(0);
        (self.normal_minutes - less).max(0)
    }

    /// The public holidays, in order.
    pub fn holidays(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        self.holidays.iter().copied()
    }

    /// The Asia/Dubai offset, UTC+04:00.
    pub fn offset() -> FixedOffset {
        FixedOffset::east_opt(DUBAI_OFFSET_SECONDS).expect("UTC+4 is a valid offset")
    }

    /// The instant `at` on the Asia/Dubai clock.
    pub fn local(&self, at: DateTime) -> ChronoDateTime<FixedOffset> {
        at.with_timezone(&Self::offset())
    }

    /// The instant of an Asia/Dubai wall-clock time.
    pub fn from_local(&self, local: NaiveDateTime) -> DateTime {
        (local - TimeDelta::seconds(i64::from(DUBAI_OFFSET_SECONDS))).and_utc()
    }

    /// Today in Asia/Dubai, from [`renox::db::now`]: the same date as
    /// [`crate::documents::today`].
    pub fn today(&self) -> NaiveDate {
        crate::documents::today()
    }

    /// Whether `date` is a working day: not Saturday, Sunday or a holiday.
    pub fn is_working_day(&self, date: NaiveDate) -> bool {
        !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) && !self.holidays.contains(&date)
    }

    /// The first working day after `date`.
    pub fn next_working_day(&self, date: NaiveDate) -> NaiveDate {
        let mut day = date;
        loop {
            day = day + Days::new(1);
            if self.is_working_day(day) {
                return day;
            }
        }
    }

    /// The instant `days` working days after `at`, at the same Asia/Dubai
    /// clock time. Working days skip the weekend and the listed holidays;
    /// working hours do not apply, so Thursday 10:00 plus one is Friday
    /// 10:00, Friday 16:00 plus one is Monday 16:00 and Saturday 10:00 plus
    /// one is Monday 10:00. Zero days give `at` itself.
    pub fn add_working_days(&self, at: DateTime, days: u32) -> DateTime {
        let local = self.local(at).naive_local();
        let mut date = local.date();
        for _ in 0..days {
            date = self.next_working_day(date);
        }
        self.from_local(date.and_time(local.time()))
    }

    /// The instant `minutes` working minutes after `from`. A start outside
    /// working hours counts from the next opening; a result that falls
    /// exactly on a closing time stays there (it does not move to the next
    /// opening). Zero or fewer minutes give `from` itself.
    pub fn add_working(&self, from: DateTime, minutes: i64) -> DateTime {
        if minutes <= 0 {
            return from;
        }
        let mut cursor = self.working_start(self.local(from).naive_local());
        let mut remaining = minutes;
        loop {
            let close = cursor.date().and_time(Self::CLOSES);
            let available = (close - cursor).num_minutes();
            if remaining <= available {
                return self.from_local(cursor + TimeDelta::minutes(remaining));
            }
            remaining -= available;
            cursor = self.next_working_day(cursor.date()).and_time(Self::OPENS);
        }
    }

    /// The working minutes from `from` to `to`; zero when `to` is not after
    /// `from`. The inverse of [`add_working`](Self::add_working):
    /// `working_minutes_between(a, add_working(a, m)) == m`.
    pub fn working_minutes_between(&self, from: DateTime, to: DateTime) -> i64 {
        if to <= from {
            return 0;
        }
        let from = self.local(from).naive_local();
        let to = self.local(to).naive_local();
        let mut total = TimeDelta::zero();
        let mut day = from.date();
        while day <= to.date() {
            if self.is_working_day(day) {
                let start = from.max(day.and_time(Self::OPENS));
                let end = to.min(day.and_time(Self::CLOSES));
                if end > start {
                    total += end - start;
                }
            }
            day = day + Days::new(1);
        }
        total.num_minutes()
    }

    /// The first working moment at or after the wall-clock time `at`.
    fn working_start(&self, at: NaiveDateTime) -> NaiveDateTime {
        let date = at.date();
        if self.is_working_day(date) {
            if at.time() < Self::OPENS {
                return date.and_time(Self::OPENS);
            }
            if at.time() < Self::CLOSES {
                return at;
            }
        }
        self.next_working_day(date).and_time(Self::OPENS)
    }
}

/// The client's calendar, provided by the kernel from `client.toml`; without
/// the kernel, the default one with no holidays.
pub fn calendar(state: &AppState) -> Arc<BusinessCalendar> {
    match state.provided::<Arc<BusinessCalendar>>() {
        Some(calendar) => Arc::clone(&calendar),
        None => Arc::new(BusinessCalendar::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::dubai_date;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    /// An Asia/Dubai wall-clock time, `2026-11-03 09:00`.
    fn dubai(calendar: &BusinessCalendar, text: &str) -> DateTime {
        let local = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a local time");
        calendar.from_local(local)
    }

    fn shown(calendar: &BusinessCalendar, at: DateTime) -> String {
        calendar.local(at).format("%Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn local_time_is_utc_plus_four() {
        let calendar = BusinessCalendar::default();
        let at: DateTime = "2026-11-03T05:00:00Z".parse().unwrap();
        assert_eq!(shown(&calendar, at), "2026-11-03 09:00");
        assert_eq!(dubai(&calendar, "2026-11-03 09:00"), at);
        assert_eq!(dubai_date(at), date("2026-11-03"));
    }

    #[test]
    fn eight_hours_within_one_day() {
        let calendar = BusinessCalendar::default();
        let due = calendar.add_working(dubai(&calendar, "2026-11-03 09:00"), 8 * 60);
        assert_eq!(shown(&calendar, due), "2026-11-03 17:00");
    }

    #[test]
    fn friday_is_a_working_day_and_the_weekend_is_skipped() {
        let calendar = BusinessCalendar::default();
        let due = calendar.add_working(dubai(&calendar, "2026-11-06 16:00"), 8 * 60);
        assert_eq!(shown(&calendar, due), "2026-11-09 15:00");
    }

    #[test]
    fn a_due_time_on_the_closing_time_stays_there() {
        let calendar = BusinessCalendar::default();
        let due = calendar.add_working(dubai(&calendar, "2026-11-03 09:00"), 27 * 60);
        assert_eq!(shown(&calendar, due), "2026-11-05 18:00");
    }

    #[test]
    fn a_start_outside_working_hours_counts_from_the_next_opening() {
        let calendar = BusinessCalendar::default();
        let evening = calendar.add_working(dubai(&calendar, "2026-11-03 20:00"), 60);
        assert_eq!(shown(&calendar, evening), "2026-11-04 10:00");
        let early = calendar.add_working(dubai(&calendar, "2026-11-04 07:30"), 60);
        assert_eq!(shown(&calendar, early), "2026-11-04 10:00");
        let saturday = calendar.add_working(dubai(&calendar, "2026-11-07 11:00"), 60);
        assert_eq!(shown(&calendar, saturday), "2026-11-09 10:00");
        let at_closing = calendar.add_working(dubai(&calendar, "2026-11-03 18:00"), 30);
        assert_eq!(shown(&calendar, at_closing), "2026-11-04 09:30");
    }

    #[test]
    fn a_listed_holiday_is_skipped() {
        let calendar = BusinessCalendar::new([date("2026-12-02"), date("2026-12-03")]);
        assert!(!calendar.is_working_day(date("2026-12-02")));
        let due = calendar.add_working(dubai(&calendar, "2026-12-01 17:00"), 2 * 60);
        assert_eq!(shown(&calendar, due), "2026-12-04 10:00");
        assert_eq!(
            calendar.next_working_day(date("2026-12-01")),
            date("2026-12-04")
        );
        assert_eq!(
            calendar.next_working_day(date("2026-12-04")),
            date("2026-12-07")
        );
    }

    #[test]
    fn working_days_keep_the_clock_time() {
        let calendar = BusinessCalendar::default();
        let due = |from: &str, days| {
            shown(
                &calendar,
                calendar.add_working_days(dubai(&calendar, from), days),
            )
        };
        assert_eq!(due("2026-11-05 10:00", 1), "2026-11-06 10:00");
        assert_eq!(due("2026-11-06 16:00", 1), "2026-11-09 16:00");
        assert_eq!(due("2026-11-07 10:00", 1), "2026-11-09 10:00");
        assert_eq!(due("2026-11-05 10:00", 2), "2026-11-09 10:00");
        assert_eq!(due("2026-11-05 10:00", 0), "2026-11-05 10:00");
    }

    #[test]
    fn working_days_use_the_dubai_date_and_skip_holidays() {
        let calendar = BusinessCalendar::new([date("2026-12-02"), date("2026-12-03")]);
        let due = |from: &str, days| {
            shown(
                &calendar,
                calendar.add_working_days(dubai(&calendar, from), days),
            )
        };
        assert_eq!(due("2026-12-01 09:30", 1), "2026-12-04 09:30");
        // 23:30 UTC on Thursday is already Friday 03:30 in Dubai.
        let at: DateTime = "2026-11-05T23:30:00Z".parse().unwrap();
        assert_eq!(
            shown(&calendar, calendar.add_working_days(at, 1)),
            "2026-11-09 03:30"
        );
    }

    #[test]
    fn working_minutes_between_is_the_inverse_of_add_working() {
        let calendar = BusinessCalendar::new([date("2026-12-02")]);
        let starts = [
            "2026-11-03 09:00",
            "2026-11-06 16:00",
            "2026-11-03 20:00",
            "2026-11-07 11:00",
            "2026-12-01 17:45",
        ];
        for start in starts {
            let from = dubai(&calendar, start);
            for minutes in [1, 59, 8 * 60, 9 * 60, 27 * 60, 126 * 60] {
                let due = calendar.add_working(from, minutes);
                assert_eq!(
                    calendar.working_minutes_between(from, due),
                    minutes,
                    "{start} + {minutes}"
                );
            }
        }
    }

    #[test]
    fn working_minutes_between_counts_only_working_hours() {
        let calendar = BusinessCalendar::default();
        let friday = dubai(&calendar, "2026-11-06 16:00");
        let monday = dubai(&calendar, "2026-11-09 11:00");
        assert_eq!(calendar.working_minutes_between(friday, monday), 4 * 60);
        assert_eq!(calendar.working_minutes_between(monday, friday), 0);
        let night = dubai(&calendar, "2026-11-09 19:00");
        let morning = dubai(&calendar, "2026-11-10 08:00");
        assert_eq!(calendar.working_minutes_between(night, morning), 0);
    }

    #[test]
    fn a_working_day_has_eight_normal_hours_and_a_rest_day_or_holiday_none() {
        let calendar = BusinessCalendar::new([date("2026-12-02")]);
        // Tuesday, Saturday, Sunday and a public holiday.
        assert_eq!(calendar.normal_minutes_on(date("2026-11-03")), 8 * 60);
        assert_eq!(calendar.normal_minutes_on(date("2026-11-07")), 0);
        assert_eq!(calendar.normal_minutes_on(date("2026-11-08")), 0);
        assert_eq!(calendar.normal_minutes_on(date("2026-12-02")), 0);
        let nine = BusinessCalendar::default().with_normal_hours(9);
        assert_eq!(nine.normal_minutes_on(date("2026-11-03")), 9 * 60);
    }

    #[test]
    fn a_reduced_hours_period_takes_hours_off_the_days_it_covers() {
        let calendar = BusinessCalendar::default().with_reduced_hours(
            date("2026-11-09"),
            date("2026-11-13"),
            2,
        );
        assert_eq!(calendar.normal_minutes_on(date("2026-11-06")), 8 * 60);
        assert_eq!(calendar.normal_minutes_on(date("2026-11-09")), 6 * 60);
        assert_eq!(calendar.normal_minutes_on(date("2026-11-13")), 6 * 60);
        assert_eq!(calendar.normal_minutes_on(date("2026-11-16")), 8 * 60);
        // The weekend inside a period stays at none.
        let long = BusinessCalendar::default().with_reduced_hours(
            date("2026-11-02"),
            date("2026-11-30"),
            2,
        );
        assert_eq!(long.normal_minutes_on(date("2026-11-07")), 0);
        // Two periods that meet: the longest cut counts, never below zero.
        let both = long.with_reduced_hours(date("2026-11-10"), date("2026-11-10"), 10);
        assert_eq!(both.normal_minutes_on(date("2026-11-10")), 0);
        assert_eq!(both.normal_minutes_on(date("2026-11-11")), 6 * 60);
        assert_eq!(both.reduced_hours().len(), 2);
    }

    #[test]
    fn zero_minutes_is_the_start_itself() {
        let calendar = BusinessCalendar::default();
        let at = dubai(&calendar, "2026-11-07 11:00");
        assert_eq!(calendar.add_working(at, 0), at);
    }
}
