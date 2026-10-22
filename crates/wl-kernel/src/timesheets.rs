//! The timesheet contracts (wl-attendance §6, §8): what other modules and the
//! verticals may ask of timesheets without depending on the `wl-attendance`
//! crate.
//!
//! - [`Timesheets`] (§6 *Provides*): the hours payroll pays at its cut-off,
//!   approved ones and, for timesheets the client has not yet approved, the
//!   accepted attendance hours marked provisional (§5.7), and the mark that a
//!   payroll run used them. `wl-attendance` provides it; `wl-hr` payroll asks
//!   it at `calculate` ([`Timesheets::hours_for_payroll`]) and tells it at
//!   `submit` ([`Timesheets::record_payroll_use`]). Billing asks for the hours
//!   the client pays ([`Timesheets::billable_hours`]) and tells it which
//!   timesheets an invoice used ([`Timesheets::record_billing_use`]). Without it,
//!   [`timesheets`] gives [`NoTimesheets`]: there are no hours.
//! - [`TimesheetClosing`]: closes the open timesheet of a context at its last
//!   day and submits it at once (09 Manpower: a deployment or a contract
//!   ends). `wl-attendance` provides it.
//! - [`ApproverResolver`] (§8): who stands behind an externally approved
//!   timesheet. The vertical provides it from the deployment's contract
//!   (09 Manpower: the client approver, the escalation contact, the account
//!   manager and the deemed-approval days). `wl-attendance` asks it for the
//!   escalation of `HRM.RUL.15` and for deemed approval. Without one,
//!   [`approver_resolver`] gives [`NoApprovers`]: nobody is known and no
//!   contract has a deemed-approval clause.

use std::collections::BTreeMap;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::attendance::{CategoryMinutes, LabourFuture};
use crate::notify::NotifyRecipient;
use crate::subject::Subject;
use crate::workforce::EmployeeId;

/// The subject type of a timesheet, `attendance.timesheet:3`.
pub const TIMESHEET_SUBJECT_TYPE: &str = "attendance.timesheet";

/// The payroll period hours are asked for (wl-hr `PayrollCalendar`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollPeriod {
    /// The first day of the period.
    pub from: NaiveDate,
    /// The last day of the period.
    pub to: NaiveDate,
    /// The payroll cut-off: attendance dated after it is not counted
    /// provisionally, and what was not yet recorded is picked up by the
    /// reconciliation when the timesheet is approved.
    pub cut_off: NaiveDate,
}

impl PayrollPeriod {
    /// The period from `from` to `to` with the cut-off on `cut_off`.
    pub fn new(from: NaiveDate, to: NaiveDate, cut_off: NaiveDate) -> Self {
        PayrollPeriod { from, to, cut_off }
    }
}

/// Hours of one category on one day, in minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoursLine {
    /// The day.
    pub date: NaiveDate,
    /// `normal`, `overtime`, `night`, `rest_day` or `public_holiday`.
    pub category: String,
    /// The minutes.
    pub minutes: i64,
}

/// The hours of one timesheet that payroll pays, as
/// [`Timesheets::hours_for_payroll`] returns them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollHours {
    /// The timesheet, `attendance.timesheet:3`.
    pub timesheet: Subject,
    /// The employee whose hours these are.
    pub owner: EmployeeId,
    /// The first day of the timesheet.
    pub period_from: NaiveDate,
    /// The last day of the timesheet.
    pub period_to: NaiveDate,
    /// The context (project, deployment...), if any.
    pub context: Option<Subject>,
    /// `internal` or `external`.
    pub approver_kind: String,
    /// Whether the hours are provisional: the timesheet is not approved yet
    /// and these are the accepted attendance hours. Payroll shows them as
    /// such.
    pub provisional: bool,
    /// The hours per day and category, oldest day first.
    pub lines: Vec<HoursLine>,
}

impl PayrollHours {
    /// The minutes per category, over all days; categories with no minutes
    /// are left out.
    pub fn totals(&self) -> Vec<CategoryMinutes> {
        let mut sums: BTreeMap<&str, i64> = BTreeMap::new();
        for line in &self.lines {
            *sums.entry(line.category.as_str()).or_default() += line.minutes;
        }
        sums.into_iter()
            .filter(|(_, minutes)| *minutes != 0)
            .map(|(category, minutes)| CategoryMinutes {
                category: category.to_owned(),
                minutes,
            })
            .collect()
    }

    /// The minutes of every category.
    pub fn total_minutes(&self) -> i64 {
        self.lines.iter().map(|line| line.minutes).sum()
    }
}

/// The timesheet service payroll uses (wl-attendance §6 `Timesheets`,
/// §5.7). `wl-attendance` provides it with [`provide_timesheets`].
///
/// Payroll asks at `calculate`, as often as it recalculates; asking has no
/// side effect and gives the same answer until the data changes. At `submit`
/// it tells the service which hours the run used. A run that is returned or
/// cancelled marks nothing.
pub trait Timesheets: Send + Sync + 'static {
    /// The hours payroll pays for `period`: those of the regular timesheets
    /// whose last day lies in it. Approved (and locked) timesheets that no
    /// payroll run used give their hours; a timesheet still `open` or
    /// `submitted` gives the accepted attendance hours up to the cut-off,
    /// marked provisional, unless the client's payroll basis for its approver
    /// is `wait`. Attendance still `flagged` is left out. Adjustment
    /// timesheets are not here: they reach payroll as `TimesheetApproved` of
    /// kind `adjustment` and as `TimesheetReconciliationNeeded`. The default
    /// has no hours.
    fn hours_for_payroll<'a>(
        &'a self,
        _state: &'a AppState,
        _period: &'a PayrollPeriod,
    ) -> LabourFuture<'a, Vec<PayrollHours>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The payroll run `run` (`hr.payroll_run:5`) used `used`, as returned
    /// by [`Timesheets::hours_for_payroll`] and as paid: an approved
    /// timesheet is `lock`ed with `paid_by` = the run; one paid on
    /// provisional hours gets `paid_provisionally_by` = the run and keeps the
    /// hours that were paid, for the reconciliation when it is approved. The
    /// same run told twice changes nothing. The default does nothing.
    fn record_payroll_use<'a>(
        &'a self,
        _state: &'a AppState,
        _run: &'a Subject,
        _used: &'a [PayrollHours],
    ) -> LabourFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// The hours the client is billed for on `timesheet`
    /// (`attendance.timesheet:3`): the entries of a regular timesheet that is
    /// `approved` or `locked` and billable, per day and category, oldest day
    /// first (`PayrollHours::provisional` is false). Hours that are paid but
    /// not billed (`permit_invalid`) are left out. `None` for a timesheet
    /// that is not a regular approved one. Asking has no side effect. The
    /// default knows none.
    fn billable_hours<'a>(
        &'a self,
        _state: &'a AppState,
        _timesheet: &'a Subject,
    ) -> LabourFuture<'a, Option<PayrollHours>> {
        Box::pin(async { Ok(None) })
    }

    /// Billing used `timesheets` for the usage invoice `reference`
    /// (`invoicing.invoice:5`): an approved one is `lock`ed with
    /// `billed_by` = the reference; one payroll locked first only gets
    /// `billed_by`. The same reference told twice changes nothing. The
    /// default does nothing.
    fn record_billing_use<'a>(
        &'a self,
        _state: &'a AppState,
        _timesheets: &'a [Subject],
        _reference: &'a str,
    ) -> LabourFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Whether `timesheet` (`attendance.timesheet:3`) is `locked`: a
    /// consumer (payroll or billing) read it, and it is never edited again
    /// (`HRM.ACL.02`, `HRM.ACL.08`). Billing asks it before it reprices
    /// hours for a rate card version dated in the past (`MPW.ACL.16`). The
    /// default says no.
    fn is_locked<'a>(
        &'a self,
        _state: &'a AppState,
        _timesheet: &'a Subject,
    ) -> LabourFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// A timesheet that payroll paid on provisional hours was approved, and the
/// approved hours differ from the hours paid (wl-attendance §5.7 point 3,
/// §6 *Events emitted*). `wl-attendance` has opened, submitted and approved
/// an adjustment timesheet of origin `provisional_reconciliation` that
/// affects payroll, and emits this and no `TimesheetApproved` of its own for
/// it, so that payroll applies the difference once: `wl-hr` puts it in the
/// next run (a negative one within the Art. 25 limits).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimesheetReconciliationNeeded {
    /// The timesheet that was approved, `attendance.timesheet:3`.
    pub timesheet: Subject,
    /// The employee whose hours these are.
    pub owner: EmployeeId,
    /// The payroll run that paid the provisional hours, `hr.payroll_run:5`.
    pub payroll_run: Subject,
    /// The hours the run paid, per category.
    pub provisional: Vec<CategoryMinutes>,
    /// The hours that were approved, per category.
    pub approved: Vec<CategoryMinutes>,
    /// Approved minus provisional, per category; categories with no
    /// difference are left out. A category can move without the total
    /// moving.
    pub delta: Vec<CategoryMinutes>,
    /// The adjustment timesheet that holds the difference per day and
    /// category, `attendance.timesheet:9`.
    pub adjustment: Subject,
}

impl Event for TimesheetReconciliationNeeded {}

/// The service when `wl-attendance` is off: there are no hours.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTimesheets;

impl Timesheets for NoTimesheets {}

/// Makes `timesheets` the app's [`Timesheets`]; `wl-attendance` calls it in
/// `register`.
pub fn provide_timesheets(app: &mut Registry, timesheets: impl Timesheets) {
    let timesheets: Arc<dyn Timesheets> = Arc::new(timesheets);
    app.provide(timesheets);
}

/// The app's [`Timesheets`], or [`NoTimesheets`] when none is provided.
pub fn timesheets(state: &AppState) -> Arc<dyn Timesheets> {
    match state.provided::<Arc<dyn Timesheets>>() {
        Some(timesheets) => Arc::clone(&timesheets),
        None => Arc::new(NoTimesheets),
    }
}

/// What closing one open timesheet came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimesheetClosure {
    /// The timesheet, `attendance.timesheet:3`.
    pub timesheet: Subject,
    /// The last day it now covers.
    pub period_to: NaiveDate,
    /// Whether it was submitted to its approver. When a check refused the
    /// submit (a day without hours allocated, attendance still flagged) it
    /// stays `open` with a task for its submitter, and `message` says why.
    pub submitted: bool,
    /// Why it was not submitted.
    pub message: Option<String>,
}

/// Closes the open timesheet of a context when the work ends (09 Manpower
/// §7.1 *Deployment* `end` and *Client contract* `end`: "each open timesheet
/// closed at the end date and submitted to the client at once"). `wl-attendance`
/// provides it with [`provide_timesheet_closing`]. Without it,
/// [`timesheet_closing`] gives [`NoTimesheetClosing`]: nothing is closed.
pub trait TimesheetClosing: Send + Sync + 'static {
    /// Closes, at `last_day`, every regular timesheet of `owner` in `context`
    /// (`manpower.deployment:4`) that is still `open` and starts on or before
    /// that day: a period that runs past `last_day` is cut to it, then the
    /// timesheet is submitted as the system, at once and not at the period
    /// end, with the checks of any submit. Timesheets already submitted,
    /// approved or locked are not touched, so a second call changes nothing.
    /// The default closes nothing.
    fn close_open<'a>(
        &'a self,
        _state: &'a AppState,
        _context: &'a Subject,
        _owner: EmployeeId,
        _last_day: NaiveDate,
    ) -> LabourFuture<'a, Vec<TimesheetClosure>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The closing when `wl-attendance` is off: nothing is closed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTimesheetClosing;

impl TimesheetClosing for NoTimesheetClosing {}

/// Makes `closing` the app's [`TimesheetClosing`]; `wl-attendance` calls it in
/// `register`.
pub fn provide_timesheet_closing(app: &mut Registry, closing: impl TimesheetClosing) {
    let closing: Arc<dyn TimesheetClosing> = Arc::new(closing);
    app.provide(closing);
}

/// The app's [`TimesheetClosing`], or [`NoTimesheetClosing`] when none is
/// provided.
pub fn timesheet_closing(state: &AppState) -> Arc<dyn TimesheetClosing> {
    match state.provided::<Arc<dyn TimesheetClosing>>() {
        Some(closing) => Arc::clone(&closing),
        None => Arc::new(NoTimesheetClosing),
    }
}

/// Who stands behind the timesheets of a context (a deployment, a project...)
/// that an external approver approves, as the context's contract says
/// (wl-attendance §8 `ApproverResolver`). The app provides it with
/// [`provide_approver_resolver`]. Every answer is for the context as it is
/// now, so a change of the contract applies to a timesheet that is waiting,
/// except the deemed-approval days, which are those in force at the `submit`
/// ([`ApproverResolver::deemed_approval_days_at`]).
pub trait ApproverResolver: Send + Sync + 'static {
    /// The user who is the account manager of the context's client: the one
    /// who collects a signed sheet and is told when the hours are deemed
    /// approved. The default has none.
    fn account_manager<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }

    /// The client contacts to tell when an approval is overdue (`HRM.RUL.15`
    /// at E days): the escalation contact of the contract. The default has
    /// none.
    fn escalation_contacts<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The client's approvers, to tell when their silence is taken as
    /// approval. The default has none.
    fn client_approvers<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The days after `submit` at which the contract deems a timesheet
    /// approved (`HRM.RUL.15` at N); `None` when the contract has no
    /// deemed-approval clause (the default). A value below 1 is no clause.
    fn deemed_approval_days<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }

    /// The days in force when the timesheet was submitted, at `submitted_at`
    /// (`HRM.RUL.15` at N: a change of the contract after the `submit` does not
    /// change a timesheet that waits, 09 §4.3). The default is
    /// [`deemed_approval_days`](ApproverResolver::deemed_approval_days), the
    /// days as they are now, for a resolver that keeps no history.
    fn deemed_approval_days_at<'a>(
        &'a self,
        state: &'a AppState,
        context: Option<&'a Subject>,
        _submitted_at: DateTime,
    ) -> LabourFuture<'a, Option<i64>> {
        self.deemed_approval_days(state, context)
    }
}

/// The resolver when none is provided: nobody is known and no contract has a
/// deemed-approval clause.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoApprovers;

impl ApproverResolver for NoApprovers {}

/// Makes `resolver` the app's [`ApproverResolver`].
pub fn provide_approver_resolver(app: &mut Registry, resolver: impl ApproverResolver) {
    let resolver: Arc<dyn ApproverResolver> = Arc::new(resolver);
    app.provide(resolver);
}

/// The app's [`ApproverResolver`], or [`NoApprovers`] when none is provided.
pub fn approver_resolver(state: &AppState) -> Arc<dyn ApproverResolver> {
    match state.provided::<Arc<dyn ApproverResolver>>() {
        Some(resolver) => Arc::clone(&resolver),
        None => Arc::new(NoApprovers),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn line(date: u32, category: &str, minutes: i64) -> HoursLine {
        HoursLine {
            date: day(date),
            category: category.into(),
            minutes,
        }
    }

    #[test]
    fn hours_add_up_per_category_and_leave_out_the_empty_ones() {
        let hours = PayrollHours {
            timesheet: Subject::new(TIMESHEET_SUBJECT_TYPE, 3),
            owner: EmployeeId(1),
            period_from: day(1),
            period_to: day(30),
            context: None,
            approver_kind: "external".into(),
            provisional: true,
            lines: vec![
                line(2, "normal", 480),
                line(3, "normal", 480),
                line(3, "overtime", 60),
                line(4, "overtime", -60),
            ],
        };
        assert_eq!(hours.total_minutes(), 960 + 60 - 60);
        assert_eq!(
            hours.totals(),
            vec![CategoryMinutes {
                category: "normal".into(),
                minutes: 960
            }]
        );
    }

    #[test]
    fn the_reconciliation_event_round_trips_through_json() {
        let event = TimesheetReconciliationNeeded {
            timesheet: Subject::new(TIMESHEET_SUBJECT_TYPE, 3),
            owner: EmployeeId(1),
            payroll_run: Subject::new("hr.payroll_run", 5),
            provisional: vec![CategoryMinutes {
                category: "normal".into(),
                minutes: 9600,
            }],
            approved: vec![CategoryMinutes {
                category: "normal".into(),
                minutes: 9120,
            }],
            delta: vec![CategoryMinutes {
                category: "normal".into(),
                minutes: -480,
            }],
            adjustment: Subject::new(TIMESHEET_SUBJECT_TYPE, 9),
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<TimesheetReconciliationNeeded>(&json).unwrap(),
            event
        );
    }

    #[test]
    fn hours_and_periods_round_trip_through_json() {
        let period = PayrollPeriod::new(day(1), day(30), day(25));
        let json = renox::serde_json::to_string(&period).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<PayrollPeriod>(&json).unwrap(),
            period
        );
    }
}
