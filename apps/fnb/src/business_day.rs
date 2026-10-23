//! The business day of an outlet (06 F&B §4.6 step 4, §7.2 *Business-day
//! cut-off*; BL-FNB-15). Late-night trading must not be split across two
//! closings: each order belongs to the trading day it was created in, and the
//! day changes at the outlet's cut-off (default 04:00, Asia/Dubai), so an order
//! at 01:30 on the 16th belongs to the 15th.
//!
//! The helpers here are pure: the cut-off is minutes after midnight, the time
//! zone is the kernel's fixed Dubai offset.

use renox::chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use renox::db::DateTime;
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;

/// The cut-off of an outlet that sets none: 04:00, in minutes after midnight.
pub const DEFAULT_CUTOFF_MINUTES: i64 = 240;

/// `at` as Asia/Dubai wall-clock time.
pub fn local(at: DateTime) -> NaiveDateTime {
    (at + Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS))).naive_utc()
}

/// The instant of the Asia/Dubai wall-clock time `wall`.
pub fn instant(wall: NaiveDateTime) -> DateTime {
    (wall - Duration::seconds(i64::from(DUBAI_OFFSET_SECONDS))).and_utc()
}

/// The business date of `at` for a cut-off of `cutoff_minutes` after midnight:
/// the local time less the cut-off, as a date. Created 03:59 with a 04:00
/// cut-off belongs to the day before; 04:00 sharp belongs to the new day.
pub fn business_date_at(at: DateTime, cutoff_minutes: i64) -> NaiveDate {
    (local(at) - Duration::minutes(cutoff_minutes)).date()
}

/// The instant the business date `date` ends and its closing may be submitted:
/// the cut-off on the next calendar day (Asia/Dubai).
pub fn closes_at(date: NaiveDate, cutoff_minutes: i64) -> DateTime {
    let next = date.succ_opt().unwrap_or(date);
    let midnight = NaiveTime::from_hms_opt(0, 0, 0).expect("midnight is a time");
    instant(next.and_time(midnight) + Duration::minutes(cutoff_minutes))
}

/// The instant of the local time `clock` (`HH:MM`) on `day`; `None` when the
/// text is not a time.
pub fn at_local(day: NaiveDate, clock: &str) -> Option<DateTime> {
    let time = NaiveTime::parse_from_str(clock.trim(), "%H:%M").ok()?;
    Some(instant(day.and_time(time)))
}

/// 240 as `04:00`.
pub fn clock_text(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// `04:00` as 240; `None` for anything that is not a time of day.
pub fn parse_clock(text: &str) -> Option<i64> {
    let time = NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()?;
    Some(i64::from(time.hour()) * 60 + i64::from(time.minute()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dubai(text: &str) -> DateTime {
        // `2026-10-16 01:30` in Dubai.
        let wall = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a time");
        instant(wall)
    }

    #[test]
    fn an_order_before_the_cut_off_belongs_to_the_day_before() {
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(
            business_date_at(dubai("2026-10-16 01:30"), 240),
            day("2026-10-15")
        );
        assert_eq!(
            business_date_at(dubai("2026-10-16 03:59"), 240),
            day("2026-10-15")
        );
        assert_eq!(
            business_date_at(dubai("2026-10-16 04:00"), 240),
            day("2026-10-16")
        );
        assert_eq!(
            business_date_at(dubai("2026-10-15 21:00"), 240),
            day("2026-10-15")
        );
    }

    #[test]
    fn a_day_closes_at_the_cut_off_of_the_next_morning() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(closes_at(day, 240), dubai("2026-10-16 04:00"));
        assert_eq!(closes_at(day, 0), dubai("2026-10-16 00:00"));
    }

    #[test]
    fn clocks_round_trip() {
        assert_eq!(clock_text(240), "04:00");
        assert_eq!(parse_clock("04:00"), Some(240));
        assert_eq!(parse_clock("23:30"), Some(1410));
        assert_eq!(parse_clock("four"), None);
    }
}
