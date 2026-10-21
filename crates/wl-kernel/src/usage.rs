//! The usage recording contract (wl-invoicing §5.2 steps 1 to 3, §6
//! `UsageRecorder`; `FIN.USG.01`): what a vertical asks of billing when the
//! hours of a timesheet have been approved, without depending on the
//! `wl-invoicing` crate.
//!
//! 09 Manpower knows which contract a deployment bills under, its role and
//! shift, the days it ran and the days it was suspended; `wl-invoicing` knows
//! the rate cards. The vertical describes the approved timesheet as a
//! [`TimesheetBilling`] and [`UsageRecorder::record_timesheet`] prices it by
//! the rate card version in force on each work date and stores the usage
//! records; [`UsageRecorder::bill`] turns what a client has not been billed
//! yet into draft invoices.
//!
//! - **Hours and days.** A line priced per hour bills the hours of each day
//!   and category; a line priced per day bills each day with hours once. A
//!   suspended day bills nothing, whatever the unit.
//! - **Months.** A line priced per month bills the calendar days the
//!   deployment ran in the timesheet's period, less the suspended ones, as
//!   the month rate times those days over the calendar days of the month
//!   (split per rate card version).
//! - **Once.** The records of one timesheet are made once: recording it
//!   again replaces the records that are not invoiced yet and leaves the
//!   invoiced ones as they are.
//!
//! Without a module that provides it, [`usage_recorder`] gives
//! [`NoUsageRecorder`]: nothing is recorded and nothing is billed.

use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;

use crate::attendance::LabourFuture;
use crate::parties::PartyId;
use crate::subject::Subject;
use crate::timesheets::HoursLine;

/// A run of calendar days, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySpan {
    /// The first day.
    pub from: NaiveDate,
    /// The last day; `None` while the span has no end.
    pub to: Option<NaiveDate>,
}

impl DaySpan {
    /// The days from `from` to `to`.
    pub fn new(from: NaiveDate, to: Option<NaiveDate>) -> Self {
        DaySpan { from, to }
    }

    /// Whether `day` lies in the span.
    pub fn contains(&self, day: NaiveDate) -> bool {
        self.from <= day && self.to.is_none_or(|to| day <= to)
    }
}

/// An approved timesheet to bill: what the vertical knows about the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimesheetBilling {
    /// The timesheet, `attendance.timesheet:3`.
    pub timesheet: Subject,
    /// The contract whose rate card prices the work,
    /// `manpower.client_contract:4`.
    pub contract: Subject,
    /// The role the rate card line is for.
    pub role: String,
    /// The shift type the rate card line is for.
    pub shift_type: String,
    /// The first day of the timesheet.
    pub period_from: NaiveDate,
    /// The last day of the timesheet.
    pub period_to: NaiveDate,
    /// The days the deployment ran (the start date to the end date).
    pub deployed: DaySpan,
    /// The days the work was suspended; nothing is billed for them.
    pub suspended: Vec<DaySpan>,
    /// The hours the client is billed, per day and category, as the
    /// timesheet's entries say ([`Timesheets::billable_hours`](crate::timesheets::Timesheets::billable_hours)).
    pub hours: Vec<HoursLine>,
}

impl TimesheetBilling {
    /// Whether `day` is a suspended one.
    pub fn is_suspended(&self, day: NaiveDate) -> bool {
        self.suspended.iter().any(|span| span.contains(day))
    }
}

/// What recording a timesheet came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageRecorded {
    /// The usage records made.
    pub records: usize,
    /// The days with hours that no accepted rate card version prices (no
    /// card, no version in force on the day, no line for the role and
    /// shift): they are not billed and someone has to look at them.
    pub unpriced: Vec<NaiveDate>,
    /// The timesheet was invoiced already: nothing was changed.
    pub invoiced: bool,
}

/// A draft invoice the billing run made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilledInvoice {
    /// The draft invoice, `invoicing.invoice:5`.
    pub invoice: Subject,
    /// Who is billed.
    pub party: PartyId,
    /// How many lines it has.
    pub lines: usize,
    /// The timesheets whose hours it bills.
    pub timesheets: Vec<Subject>,
}

/// Records priced usage and bills it (wl-invoicing §6 `UsageRecorder`).
/// `wl-invoicing` provides it with [`provide_usage_recorder`].
pub trait UsageRecorder: Send + Sync + 'static {
    /// Prices the approved hours of `billing` by the rate card of its
    /// contract and stores them as unbilled usage records. Recording the same
    /// timesheet again replaces the records not invoiced yet, so a retry
    /// bills nothing twice. The default records nothing.
    fn record_timesheet<'a>(
        &'a self,
        _state: &'a AppState,
        _billing: &'a TimesheetBilling,
    ) -> LabourFuture<'a, UsageRecorded> {
        Box::pin(async { Ok(UsageRecorded::default()) })
    }

    /// Turns the unbilled usage of `party` into draft invoices, one per
    /// issuing entity and currency, and marks the timesheets they bill as
    /// used by billing. A party with nothing unbilled gets none. The default
    /// bills nothing.
    fn bill<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
    ) -> LabourFuture<'a, Vec<BilledInvoice>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The recorder when no module provides one: nothing is recorded or billed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUsageRecorder;

impl UsageRecorder for NoUsageRecorder {}

/// Makes `recorder` the app's [`UsageRecorder`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_usage_recorder(app: &mut Registry, recorder: impl UsageRecorder) {
    let recorder: Arc<dyn UsageRecorder> = Arc::new(recorder);
    app.provide(recorder);
}

/// The app's [`UsageRecorder`], or [`NoUsageRecorder`] when none is provided.
pub fn usage_recorder(state: &AppState) -> Arc<dyn UsageRecorder> {
    match state.provided::<Arc<dyn UsageRecorder>>() {
        Some(recorder) => Arc::clone(&recorder),
        None => Arc::new(NoUsageRecorder),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn a_span_without_an_end_runs_on() {
        let open = DaySpan::new(day(21), None);
        assert!(!open.contains(day(20)));
        assert!(open.contains(day(21)));
        assert!(open.contains(day(30)));
    }

    #[test]
    fn a_span_holds_both_its_ends() {
        let span = DaySpan::new(day(5), Some(day(7)));
        assert!(!span.contains(day(4)));
        assert!(span.contains(day(5)) && span.contains(day(7)));
        assert!(!span.contains(day(8)));
    }
}
