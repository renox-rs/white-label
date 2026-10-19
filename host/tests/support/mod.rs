//! Shared by the host tests that send notifications.

use renox::chrono::NaiveDateTime;
use wl_kernel::BusinessCalendar;
use wl_notify::Clock;

/// The notify module's clock fixed at noon in Dubai: inside every send window
/// and outside the quiet hours (21:00-08:00, `NTF.RUL.02`), so a send does not
/// depend on the hour the test runs at.
pub fn noon_clock() -> Clock {
    let local = NaiveDateTime::parse_from_str("2026-10-19 12:00", "%Y-%m-%d %H:%M")
        .expect("a date and time");
    Clock::fixed(BusinessCalendar::default().from_local(local))
}
