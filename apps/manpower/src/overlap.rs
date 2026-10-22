//! No two deployments of one worker at the same hours (09 Manpower §3.3
//! `MPW.ACL.15`, §4.1 *The worker is already deployed for the same hours*,
//! BL-MPW-10).
//!
//! `activate` and `extend` of a deployment are refused when the worker already
//! has an `active` or `suspended` deployment whose shifts overlap it. A
//! `planned` deployment is not compared (it is checked when it activates), and
//! an `ended` or `cancelled` one never blocks. Overlapping attendance is
//! flagged separately by wl-attendance.
//!
//! - **The test.** The date ranges intersect and, on at least one day both
//!   work, the shift windows intersect. A deployment works the weekdays of
//!   its `work_days` (every day when none is given) between its start and end
//!   dates; no end date is open-ended. A window is the shift start to the shift
//!   end; one that ends at or before it starts crosses midnight (22:00 to
//!   06:00 occupies the evening of its day and the morning of the next), and a
//!   deployment without shift times is there all day. Windows that touch (one
//!   ends 14:00, the other starts 14:00) do not overlap. The range checked of
//!   the deployment being activated or extended starts on the day of the
//!   action at the earliest: the days gone cannot be changed.
//! - **Concurrency.** The check and the state change run under a lock on the
//!   employee ([`take`], one at a time in this process) and the employee's
//!   deployments are locked for update in the transaction ([`problem`], on
//!   PostgreSQL), so the second of two parallel activations of one worker sees
//!   the first and is refused.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::db::Transaction;
use renox::prelude::*;
use renox::tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::deployment::{Deployment, DeploymentStatus};
use crate::site::Site;
use crate::texts::english;

type Locks = Mutex<HashMap<i64, Arc<AsyncMutex<()>>>>;

fn locks() -> &'static Locks {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    LOCKS.get_or_init(Mutex::default)
}

/// The lock of one employee's deployments, released when dropped.
#[derive(Debug)]
pub struct EmployeeLock(#[allow(dead_code)] OwnedMutexGuard<()>);

/// Takes the lock of `employee`, waiting for any `activate` or `extend` of the
/// same worker that holds it.
pub async fn take(employee: i64) -> EmployeeLock {
    let mutex = {
        let mut table = locks().lock().unwrap_or_else(|e| e.into_inner());
        table.retain(|_, m| Arc::strong_count(m) > 1);
        Arc::clone(table.entry(employee).or_default())
    };
    EmployeeLock(mutex.lock_owned().await)
}

/// The weekday `text` names (`mon`), if it does.
fn weekday(text: &str) -> Option<Weekday> {
    match text.trim().to_lowercase().as_str() {
        "mon" => Some(Weekday::Mon),
        "tue" => Some(Weekday::Tue),
        "wed" => Some(Weekday::Wed),
        "thu" => Some(Weekday::Thu),
        "fri" => Some(Weekday::Fri),
        "sat" => Some(Weekday::Sat),
        "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// The working days of `text` (`mon,tue,fri`); none when the text is not a
/// list of weekdays.
pub fn parse_days(text: &str) -> Option<BTreeSet<u32>> {
    let mut days = BTreeSet::new();
    for part in text.split(',') {
        days.insert(weekday(part)?.num_days_from_monday());
    }
    Some(days)
}

/// `07:30` as minutes since midnight.
pub fn parse_time(text: &str) -> Option<i64> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    ((0..24).contains(&hours) && (0..60).contains(&minutes)).then_some(hours * 60 + minutes)
}

/// The days a deployment works: every day when it names none.
fn works(deployment: &Deployment, date: NaiveDate) -> bool {
    match deployment.work_days.as_deref().and_then(parse_days) {
        Some(days) if !days.is_empty() => days.contains(&date.weekday().num_days_from_monday()),
        _ => true,
    }
}

/// The window of the shift in minutes from midnight: `(start, length)`; all day
/// without times.
fn window(deployment: &Deployment) -> (i64, i64) {
    let start = deployment.shift_start.as_deref().and_then(parse_time);
    let end = deployment.shift_end.as_deref().and_then(parse_time);
    match (start, end) {
        (Some(start), Some(end)) if end > start => (start, end - start),
        // Ends at or before it starts: through midnight (equal: a full day).
        (Some(start), Some(end)) => (start, end + 24 * 60 - start),
        _ => (0, 24 * 60),
    }
}

/// Whether `a` working `a_range` and `b` working `b_range` (dates, `None` for
/// no end) have shifts that intersect on a day both work.
fn overlaps(
    a: &Deployment,
    a_range: (NaiveDate, Option<NaiveDate>),
    b: &Deployment,
    b_range: (NaiveDate, Option<NaiveDate>),
) -> bool {
    let low = a_range.0.max(b_range.0);
    let high = match (a_range.1, b_range.1) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    };
    if high.is_some_and(|high| high < low) {
        return false;
    }
    // Two weeks hold every weekday pattern; the day before catches a shift
    // that crosses midnight into the first common day.
    let first = low.checked_sub_days(Days::new(1)).unwrap_or(low);
    let last = low + Days::new(14);
    let last = high.map_or(last, |high| high.min(last));
    let (a_start, a_len) = window(a);
    let (b_start, b_len) = window(b);
    let within = |range: (NaiveDate, Option<NaiveDate>), date: NaiveDate| {
        date >= range.0 && range.1.is_none_or(|end| date <= end)
    };
    let mut date = first;
    while date <= last {
        if within(a_range, date) && works(a, date) {
            let a_from = a_start;
            let a_to = a_start + a_len;
            // The same day, and the next day's own windows, in the axis of
            // `date` (minutes since its midnight).
            for offset in [-1_i64, 0, 1] {
                let other = if offset < 0 {
                    date.checked_sub_days(Days::new(1))
                } else {
                    date.checked_add_days(Days::new(offset.unsigned_abs()))
                };
                let Some(other) = other else { continue };
                if within(b_range, other) && works(b, other) {
                    let shift = offset * 24 * 60;
                    let b_from = b_start + shift;
                    let b_to = b_from + b_len;
                    if a_from < b_to && b_from < a_to {
                        return true;
                    }
                }
            }
        }
        date = date + Days::new(1);
    }
    false
}

/// Why `deployment`, over `from` to `to` (`None`: open-ended), cannot be
/// activated or extended: the line naming the `active` or `suspended`
/// deployment of the same worker whose shifts overlap it. The worker's
/// deployments are locked for update in `tx`.
///
/// # Errors
///
/// The database's errors.
pub async fn problem(
    tx: &mut Transaction,
    deployment: &Deployment,
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Result<Option<String>> {
    if to.is_some_and(|to| to < from) {
        return Ok(None);
    }
    let others = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(deployment.employee_id))
        .where_(Deployment::ID.ne(deployment.id))
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .order_by(Deployment::ID)
        .lock_for_update()
        .get(&mut *tx)
        .await?;
    let this = (from.max(deployment.start_date), to);
    for other in others {
        if overlaps(deployment, this, &other, (other.start_date, other.end_date)) {
            let site = Site::find(&mut *tx, other.site_id)
                .await?
                .map(|site| site.name)
                .unwrap_or_default();
            let ends = other
                .end_date
                .map_or_else(|| "-".to_owned(), |end| end.to_string());
            let shift = format!(
                "{}-{}",
                other.shift_start.as_deref().unwrap_or("00:00"),
                other.shift_end.as_deref().unwrap_or("24:00")
            );
            return Ok(Some(english(
                "manpower.deployment.overlap",
                &[
                    ("site", &site),
                    ("id", &other.id),
                    ("status", &other.status.as_str()),
                    ("role", &other.role),
                    ("from", &other.start_date),
                    ("until", &ends),
                    ("shift", &shift),
                ],
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, day: u32) -> NaiveDate {
        // 2026-11-02 is a Monday.
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    fn shift(days: Option<&str>, from: &str, to: &str) -> Deployment {
        Deployment {
            work_days: days.map(str::to_owned),
            shift_start: Some(from.into()),
            shift_end: Some(to.into()),
            ..Deployment::default()
        }
    }

    const OPEN: (NaiveDate, Option<NaiveDate>) = (NaiveDate::MIN, None);

    #[test]
    fn windows_that_intersect_overlap_and_windows_that_touch_do_not() {
        let a = shift(None, "07:00", "15:00");
        assert!(overlaps(&a, OPEN, &shift(None, "14:00", "22:00"), OPEN));
        assert!(!overlaps(&a, OPEN, &shift(None, "15:00", "23:00"), OPEN));
        assert!(!overlaps(&a, OPEN, &shift(None, "00:00", "07:00"), OPEN));
    }

    #[test]
    fn only_a_day_both_work_counts() {
        let a = shift(Some("mon,tue,wed,thu,fri,sat"), "07:00", "15:00");
        let sundays = shift(Some("sun"), "08:00", "12:00");
        let fridays = shift(Some("fri"), "08:00", "12:00");
        assert!(!overlaps(&a, OPEN, &sundays, OPEN));
        assert!(overlaps(&a, OPEN, &fridays, OPEN));
    }

    #[test]
    fn date_ranges_that_do_not_meet_do_not_overlap() {
        let a = shift(None, "07:00", "15:00");
        let b = shift(None, "07:00", "15:00");
        let first = (day(11, 2), Some(day(11, 30)));
        let second = (day(12, 1), None);
        assert!(!overlaps(&a, first, &b, second));
        assert!(overlaps(&a, (day(11, 2), Some(day(12, 1))), &b, second));
    }

    #[test]
    fn an_overnight_shift_reaches_into_the_next_morning() {
        let night = shift(None, "22:00", "06:00");
        assert!(overlaps(&night, OPEN, &shift(None, "05:00", "09:00"), OPEN));
        assert!(!overlaps(
            &night,
            OPEN,
            &shift(None, "06:00", "14:00"),
            OPEN
        ));
        assert!(overlaps(&night, OPEN, &shift(None, "21:00", "23:00"), OPEN));
        // A night on Friday ends Saturday morning: a Saturday day shift meets it,
        // a Sunday one does not.
        let friday = shift(Some("fri"), "22:00", "06:00");
        assert!(overlaps(
            &friday,
            OPEN,
            &shift(Some("sat"), "04:00", "08:00"),
            OPEN
        ));
        assert!(!overlaps(
            &friday,
            OPEN,
            &shift(Some("sun"), "04:00", "08:00"),
            OPEN
        ));
    }

    #[test]
    fn a_deployment_without_shift_times_is_there_all_day() {
        let all_day = Deployment::default();
        assert!(overlaps(
            &all_day,
            OPEN,
            &shift(None, "03:00", "04:00"),
            OPEN
        ));
    }

    #[test]
    fn days_and_times_are_read_strictly() {
        assert_eq!(parse_days("mon, Fri").unwrap().len(), 2);
        assert!(parse_days("mon,funday").is_none());
        assert_eq!(parse_time("07:30"), Some(450));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("7"), None);
    }
}
