//! The payroll calendar contract (wl-hr §6 *Provides*, `PayrollCalendar`): the
//! dates of a payroll period that depend on the company, not on the code.
//!
//! Today that is the WPS due date of a period, the day by which the salary of
//! a month has to be paid (MR 340/2026). The default is the 1st of the month
//! after the period ([`DefaultPayrollCalendar`]); a client whose entities
//! differ provides its own with [`provide_payroll_calendar`]. Without one,
//! [`payroll_calendar`] gives the default. `wl-hr` asks it for the escalation
//! of a rejected WPS file (`HRM.RUL.11`) and for the warning that a carried
//! line pays late.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;

/// The future a [`PayrollCalendar`] answers with.
pub type CalendarFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The dates of a payroll period, per legal entity.
pub trait PayrollCalendar: Send + Sync + 'static {
    /// The WPS due date of `period` (`2026-11`) for `legal_entity_id` (`None`
    /// = the default entity): the day the salary has to be paid by. `None`
    /// when `period` is not a month.
    fn wps_due_date<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: Option<i64>,
        period: &'a str,
    ) -> CalendarFuture<'a, Option<NaiveDate>>;
}

/// The 1st of the month after `period` (`2026-11` gives 2026-12-01), the
/// default WPS due date. `None` when `period` is not a month.
pub fn first_of_next_month(period: &str) -> Option<NaiveDate> {
    let (year, month) = period.split_once('-')?;
    let (year, month): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
    if !(1..=12).contains(&month) {
        return None;
    }
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
}

/// The calendar when none is provided: the WPS due date is the 1st of the
/// month after the period, for every entity.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultPayrollCalendar;

impl PayrollCalendar for DefaultPayrollCalendar {
    fn wps_due_date<'a>(
        &'a self,
        _state: &'a AppState,
        _legal_entity_id: Option<i64>,
        period: &'a str,
    ) -> CalendarFuture<'a, Option<NaiveDate>> {
        Box::pin(async move { Ok(first_of_next_month(period)) })
    }
}

/// Makes `calendar` the app's [`PayrollCalendar`].
pub fn provide_payroll_calendar(app: &mut Registry, calendar: impl PayrollCalendar) {
    let calendar: Arc<dyn PayrollCalendar> = Arc::new(calendar);
    app.provide(calendar);
}

/// The app's [`PayrollCalendar`], or [`DefaultPayrollCalendar`] when none is
/// provided.
pub fn payroll_calendar(state: &AppState) -> Arc<dyn PayrollCalendar> {
    match state.provided::<Arc<dyn PayrollCalendar>>() {
        Some(calendar) => Arc::clone(&calendar),
        None => Arc::new(DefaultPayrollCalendar),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_default_due_date_is_the_first_of_the_following_month() {
        assert_eq!(first_of_next_month("2026-11"), Some(day(2026, 12, 1)));
        assert_eq!(first_of_next_month("2026-12"), Some(day(2027, 1, 1)));
        assert_eq!(first_of_next_month("2026-13"), None);
        assert_eq!(first_of_next_month("november"), None);
    }
}
