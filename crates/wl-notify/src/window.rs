//! The marketing send window and the quiet hours (wl-notify §7, `NTF.RUL.02`).
//!
//! Pure rules, no database: a [`Window`] is a same-day span in minutes after
//! midnight Asia/Dubai (UTC+04:00 all year, so no daylight saving). A
//! marketing message goes out only inside its channel's window; a normal
//! urgency message to an external recipient is not sent in the
//! [`QUIET_HOURS`].

use std::fmt;

use renox::chrono::{Days, NaiveTime, TimeDelta, Timelike};
use renox::db::DateTime;
use wl_kernel::BusinessCalendar;
use wl_kernel::notify::NotifyChannel;

use crate::message::QUIET_HOURS;

/// Minutes in a day.
const DAY: u32 = 24 * 60;
/// The default window opens at 07:00.
pub const DEFAULT_START: u32 = 7 * 60;
/// The default window closes at 21:00.
pub const DEFAULT_END: u32 = 21 * 60;

/// The text of the refusal of an SMS window outside 07:00-21:00.
pub const SMS_LIMIT_MESSAGE: &str = "SMS marketing may only be sent 07:00-21:00";
/// The text of the refusal of a window that does not start before it ends.
pub const ORDER_MESSAGE: &str = "The window must start before it ends";

/// A daily span, `start` inclusive and `end` exclusive, in minutes after
/// midnight Asia/Dubai. Same-day only: `start < end <= 24:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// The first minute inside the window.
    pub start: u32,
    /// The first minute after the window.
    pub end: u32,
}

impl Default for Window {
    /// 07:00-21:00.
    fn default() -> Self {
        Self {
            start: DEFAULT_START,
            end: DEFAULT_END,
        }
    }
}

/// Why a window was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// `start` is not before `end`, or a time is past the end of the day.
    Order,
    /// An SMS window reaches outside 07:00-21:00.
    SmsLimit,
}

impl WindowError {
    /// The message shown with the 422.
    pub fn message(self) -> &'static str {
        match self {
            WindowError::Order => ORDER_MESSAGE,
            WindowError::SmsLimit => SMS_LIMIT_MESSAGE,
        }
    }
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for WindowError {}

/// The minutes after midnight Asia/Dubai of `at`.
fn minute_of_day(at: DateTime) -> u32 {
    let local = BusinessCalendar::default().local(at);
    local.hour() * 60 + local.minute()
}

/// The instant of `minute` on the Dubai day of `at`, `days` days later.
/// `minute` 1440 is midnight of the next day.
fn at_minute(at: DateTime, minute: u32, days: u64) -> DateTime {
    let calendar = BusinessCalendar::default();
    let date = calendar.local(at).date_naive() + Days::new(days);
    let midnight = date.and_time(NaiveTime::MIN);
    calendar.from_local(midnight + TimeDelta::minutes(i64::from(minute)))
}

/// Whether `at` is inside `window`: 07:00 is, 21:00 is not (default window).
pub fn in_window(window: Window, at: DateTime) -> bool {
    let minute = minute_of_day(at);
    window.start <= minute && minute < window.end
}

/// When `window` opens next: `at` itself when inside, the start today when
/// before it, the start tomorrow when at or after the end.
pub fn next_opening(window: Window, at: DateTime) -> DateTime {
    let minute = minute_of_day(at);
    if window.start <= minute && minute < window.end {
        at
    } else if minute < window.start {
        at_minute(at, window.start, 0)
    } else {
        at_minute(at, window.start, 1)
    }
}

/// Whether `at` is inside the quiet hours (21:00-08:00, across midnight).
pub fn in_quiet_hours(at: DateTime) -> bool {
    let (start, end) = QUIET_HOURS;
    let minute = minute_of_day(at);
    minute >= start || minute < end
}

/// When the quiet hours end next: the next 08:00 for `at` inside them, `at`
/// itself outside them.
pub fn next_quiet_end(at: DateTime) -> DateTime {
    let (start, end) = QUIET_HOURS;
    let minute = minute_of_day(at);
    if minute >= start {
        at_minute(at, end, 1)
    } else if minute < end {
        at_minute(at, end, 0)
    } else {
        at
    }
}

/// Checks a window to be saved for `channel`: it must start before it ends
/// within the day, and an SMS window must stay inside 07:00-21:00 (email and
/// WhatsApp may be widened to any window within the day).
pub fn validate(channel: NotifyChannel, window: Window) -> Result<(), WindowError> {
    if window.start >= window.end || window.end > DAY {
        return Err(WindowError::Order);
    }
    if channel == NotifyChannel::Sms && (window.start < DEFAULT_START || window.end > DEFAULT_END) {
        return Err(WindowError::SmsLimit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDateTime;

    /// A Dubai wall-clock time, `2026-10-19 06:59`, as an instant.
    fn dubai(text: &str) -> DateTime {
        let local = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("a date and time");
        BusinessCalendar::default().from_local(local)
    }

    #[test]
    fn the_window_opens_at_0700_and_closes_at_2100() {
        let w = Window::default();
        assert!(!in_window(w, dubai("2026-10-19 06:59")));
        assert!(in_window(w, dubai("2026-10-19 07:00")));
        assert!(in_window(w, dubai("2026-10-19 20:59")));
        assert!(!in_window(w, dubai("2026-10-19 21:00")));
    }

    #[test]
    fn the_next_opening_is_today_when_before_and_tomorrow_when_after() {
        let w = Window::default();
        let noon = dubai("2026-10-19 12:00");
        assert_eq!(next_opening(w, noon), noon);
        assert_eq!(
            next_opening(w, dubai("2026-10-19 06:59")),
            dubai("2026-10-19 07:00")
        );
        assert_eq!(
            next_opening(w, dubai("2026-10-19 21:00")),
            dubai("2026-10-20 07:00")
        );
        assert_eq!(
            next_opening(w, dubai("2026-10-19 22:15")),
            dubai("2026-10-20 07:00")
        );
    }

    #[test]
    fn the_next_opening_rolls_over_a_month_and_uses_dubai_midnight() {
        let w = Window::default();
        assert_eq!(
            next_opening(w, dubai("2026-10-31 23:30")),
            dubai("2026-11-01 07:00")
        );
        // 21:30 UTC is already 01:30 the next day in Dubai.
        let late_utc = dubai("2026-10-19 21:30") + TimeDelta::hours(4);
        assert_eq!(next_opening(w, late_utc), dubai("2026-10-20 07:00"));
    }

    #[test]
    fn a_narrowed_window_defers_to_its_own_start() {
        let w = Window {
            start: 9 * 60,
            end: 20 * 60,
        };
        assert_eq!(
            next_opening(w, dubai("2026-10-19 20:30")),
            dubai("2026-10-20 09:00")
        );
    }

    #[test]
    fn the_quiet_hours_run_from_2100_to_0800_across_midnight() {
        assert!(!in_quiet_hours(dubai("2026-10-19 20:59")));
        assert!(in_quiet_hours(dubai("2026-10-19 21:00")));
        assert!(in_quiet_hours(dubai("2026-10-19 00:00")));
        assert!(in_quiet_hours(dubai("2026-10-19 07:59")));
        assert!(!in_quiet_hours(dubai("2026-10-19 08:00")));
    }

    #[test]
    fn the_quiet_hours_end_at_the_next_0800() {
        let noon = dubai("2026-10-19 12:00");
        assert_eq!(next_quiet_end(noon), noon);
        assert_eq!(
            next_quiet_end(dubai("2026-10-19 22:15")),
            dubai("2026-10-20 08:00")
        );
        assert_eq!(
            next_quiet_end(dubai("2026-10-19 21:00")),
            dubai("2026-10-20 08:00")
        );
        assert_eq!(
            next_quiet_end(dubai("2026-10-19 03:00")),
            dubai("2026-10-19 08:00")
        );
        assert_eq!(
            next_quiet_end(dubai("2026-10-19 07:59")),
            dubai("2026-10-19 08:00")
        );
    }

    #[test]
    fn an_sms_window_stays_inside_0700_2100() {
        let sms = NotifyChannel::Sms;
        let w = |start, end| Window { start, end };
        assert_eq!(validate(sms, w(360, 1320)), Err(WindowError::SmsLimit));
        assert_eq!(validate(sms, w(360, 1200)), Err(WindowError::SmsLimit));
        assert_eq!(validate(sms, w(480, 1320)), Err(WindowError::SmsLimit));
        assert_eq!(validate(sms, w(540, 1200)), Ok(()));
        assert_eq!(validate(sms, Window::default()), Ok(()));
        assert_eq!(
            WindowError::SmsLimit.message(),
            "SMS marketing may only be sent 07:00-21:00"
        );
    }

    #[test]
    fn email_and_whatsapp_may_be_widened_but_not_inverted() {
        let w = |start, end| Window { start, end };
        for channel in [NotifyChannel::Email, NotifyChannel::Whatsapp] {
            assert_eq!(validate(channel, w(360, 1380)), Ok(()));
            assert_eq!(validate(channel, w(0, 1440)), Ok(()));
            assert_eq!(validate(channel, w(540, 540)), Err(WindowError::Order));
            assert_eq!(validate(channel, w(600, 540)), Err(WindowError::Order));
            assert_eq!(validate(channel, w(60, 1441)), Err(WindowError::Order));
        }
    }
}
