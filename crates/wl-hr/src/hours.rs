//! The hours of a payroll period and what they pay (wl-hr §5.3 step 2, §7
//! *Overtime rates by category*).
//!
//! | Category | Rate on the hourly rate |
//! |---|---|
//! | `normal` | 100% |
//! | `overtime` | 125% |
//! | `night`, `rest_day` | 150% |
//! | `public_holiday` | 150% (decision: the spec lists no rate; it is paid as the rest day) |
//!
//! A salaried employee's `normal` hours are inside the fixed wage, so only the
//! other categories add to a line; the `normal` hours of a difference (a
//! reconciliation, an adjustment timesheet) are paid or taken back at 100%.

use renox::chrono::{Datelike, Duration, NaiveDate};
use wl_kernel::timesheets::PayrollPeriod;

/// The category of hours the fixed wage already pays.
pub const NORMAL: &str = "normal";

/// The percent of the hourly rate hours of `category` are paid at.
pub fn rate_percent(category: &str) -> i64 {
    match category {
        "overtime" => 125,
        "night" | "rest_day" | "public_holiday" => 150,
        _ => 100,
    }
}

/// What `minutes` (negative to take back) of `category` are worth at `hourly`
/// fils an hour, rounded to the nearest fils (halves away from zero).
pub fn amount_for(minutes: i64, hourly: i64, category: &str) -> i64 {
    let scaled = i128::from(minutes) * i128::from(hourly) * i128::from(rate_percent(category));
    let divisor = 60 * 100;
    let half = divisor / 2;
    let rounded = if scaled >= 0 {
        (scaled + half) / divisor
    } else {
        (scaled - half) / divisor
    };
    i64::try_from(rounded).unwrap_or(0)
}

/// The first and the last day of `period` (`2026-11`); `None` when it is not a
/// month.
pub fn period_bounds(period: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (year, month) = period.split_once('-')?;
    let (year, month): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
    let from = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some((from, next - Duration::days(1)))
}

/// A timesheet's period for a list: `October 2026` for a whole month, else
/// `01 Oct 2026 - 15 Oct 2026`.
pub fn month_text(from: NaiveDate, to: NaiveDate) -> String {
    match period_bounds(&from.format("%Y-%m").to_string()) {
        Some((first, last)) if first == from && last == to => from.format("%B %Y").to_string(),
        _ => format!("{} - {}", from.format("%d %b %Y"), to.format("%d %b %Y")),
    }
}

/// The period asked of the timesheets: the month with the cut-off on
/// `cut_off_day` (or the last day of a shorter month).
pub fn payroll_period(period: &str, cut_off_day: u32) -> Option<PayrollPeriod> {
    let (from, to) = period_bounds(period)?;
    let cut_off = from.with_day(cut_off_day).unwrap_or(to).min(to);
    Some(PayrollPeriod::new(from, to, cut_off))
}

/// The month after `period`, as `2026-12`.
pub fn next_period(period: &str) -> Option<String> {
    let (_, to) = period_bounds(period)?;
    let next = to + Duration::days(1);
    Some(format!("{:04}-{:02}", next.year(), next.month()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn four_overtime_hours_at_125_percent_of_15_dirhams_pay_75() {
        assert_eq!(amount_for(240, 1_500, "overtime"), 7_500);
    }

    #[test]
    fn hours_taken_back_are_negative_and_normal_hours_are_paid_at_the_plain_rate() {
        assert_eq!(amount_for(-480, 1_500, "normal"), -12_000);
        assert_eq!(amount_for(-3_000, 1_500, "normal"), -75_000);
    }

    #[test]
    fn night_and_rest_day_hours_are_paid_at_150_percent() {
        assert_eq!(amount_for(60, 1_000, "night"), 1_500);
        assert_eq!(amount_for(60, 1_000, "rest_day"), 1_500);
    }

    #[test]
    fn an_amount_is_rounded_to_the_nearest_fils() {
        // 1 minute at 10.00 an hour is 16.67 fils.
        assert_eq!(amount_for(1, 1_000, "normal"), 17);
        assert_eq!(amount_for(-1, 1_000, "normal"), -17);
    }

    #[test]
    fn a_period_is_a_month_with_its_cut_off() {
        let period = payroll_period("2026-11", 25).unwrap();
        assert_eq!(
            (period.from, period.to, period.cut_off),
            (day(2026, 11, 1), day(2026, 11, 30), day(2026, 11, 25))
        );
        let february = payroll_period("2027-02", 30).unwrap();
        assert_eq!(february.cut_off, day(2027, 2, 28));
        assert!(payroll_period("november", 25).is_none());
    }

    #[test]
    fn the_next_period_follows_the_year_end() {
        assert_eq!(next_period("2026-11").as_deref(), Some("2026-12"));
        assert_eq!(next_period("2026-12").as_deref(), Some("2027-01"));
    }
}
