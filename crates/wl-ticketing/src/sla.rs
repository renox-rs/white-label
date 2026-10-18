//! The calendar the SLA counts working time in (wl-ticketing §8).
//!
//! [`SlaCalendar`] is a ticketing trait whose default is the kernel's
//! [`BusinessCalendar`]; an app or vertical provides another one (Ramadan
//! hours, a site calendar) with [`provide_sla_calendar`].

use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::BusinessCalendar;

/// Working time for SLA due dates.
pub trait SlaCalendar: Send + Sync + 'static {
    /// The instant `minutes` working minutes after `from`.
    fn add_working(&self, from: DateTime, minutes: i64) -> DateTime;

    /// The working minutes from `from` to `to`; zero when `to` is not after
    /// `from`.
    fn working_minutes_between(&self, from: DateTime, to: DateTime) -> i64;
}

impl SlaCalendar for BusinessCalendar {
    fn add_working(&self, from: DateTime, minutes: i64) -> DateTime {
        BusinessCalendar::add_working(self, from, minutes)
    }

    fn working_minutes_between(&self, from: DateTime, to: DateTime) -> i64 {
        BusinessCalendar::working_minutes_between(self, from, to)
    }
}

/// Makes `calendar` the SLA calendar, in place of the kernel's.
pub fn provide_sla_calendar(app: &mut Registry, calendar: impl SlaCalendar) {
    let calendar: Arc<dyn SlaCalendar> = Arc::new(calendar);
    app.provide(calendar);
}

/// The app's SLA calendar: the one provided with [`provide_sla_calendar`],
/// else the kernel's client calendar.
pub fn sla_calendar(state: &AppState) -> Arc<dyn SlaCalendar> {
    match state.provided::<Arc<dyn SlaCalendar>>() {
        Some(calendar) => Arc::clone(&calendar),
        None => wl_kernel::calendar::calendar(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn dubai(date: &str, hour: u32) -> DateTime {
        let local = date
            .parse::<NaiveDate>()
            .expect("a date")
            .and_hms_opt(hour, 0, 0)
            .expect("a time");
        BusinessCalendar::default().from_local(local)
    }

    #[test]
    fn the_kernel_calendar_is_a_sla_calendar() {
        let calendar: Arc<dyn SlaCalendar> = Arc::new(BusinessCalendar::default());
        // Tuesday 2026-11-03 10:00 + 2 working hours.
        let from = dubai("2026-11-03", 10);
        let due = calendar.add_working(from, 120);
        assert_eq!(due, dubai("2026-11-03", 12));
        assert_eq!(calendar.working_minutes_between(from, due), 120);
    }

    #[test]
    fn working_time_skips_the_weekend() {
        let calendar: Arc<dyn SlaCalendar> = Arc::new(BusinessCalendar::default());
        // Friday 2026-11-06 17:00 + 2 h: 1 h that day, 1 h on Monday (Sat and Sun are the weekend).
        let due = calendar.add_working(dubai("2026-11-06", 17), 120);
        assert_eq!(due, dubai("2026-11-09", 10));
    }
}
