//! The attendance contract (wl-attendance §6 *Events emitted*): what other
//! modules and the verticals may know about attendance records without
//! depending on the `wl-attendance` crate.
//!
//! `wl-attendance` emits [`AttendanceRecorded`] when a check-in, a check-out
//! or a supervisor marking is saved, and [`AttendanceFlagged`] once for every
//! flag it adds to a record (`overlap`, `overtime_cap`, `midday_break`,
//! `permit_invalid`, ...). A record is referred to by [`AttendanceRecordId`]
//! or as a [`Subject`] of type [`ATTENDANCE_SUBJECT_TYPE`].

use std::fmt;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;
use crate::workforce::EmployeeId;

/// The subject type of an attendance record, for [`Subject`] references.
pub const ATTENDANCE_SUBJECT_TYPE: &str = "attendance.record";

/// The subject type of a work location.
pub const WORK_LOCATION_SUBJECT_TYPE: &str = "attendance.work_location";

/// The id of an attendance record.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct AttendanceRecordId(pub i64);

impl AttendanceRecordId {
    /// The record as a [`Subject`]: `attendance.record:7`.
    pub fn subject(self) -> Subject {
        Subject::new(ATTENDANCE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for AttendanceRecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for AttendanceRecordId {
    fn from(id: i64) -> Self {
        AttendanceRecordId(id)
    }
}

/// An attendance record was saved: a check-in, a check-out or a supervisor
/// marking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendanceRecorded {
    /// The record.
    pub record: AttendanceRecordId,
    /// The employee.
    pub employee: EmployeeId,
    /// The work location, `attendance.work_location:3`.
    pub location: Subject,
    /// The day, in Asia/Dubai.
    pub date: NaiveDate,
    /// `supervisor`, `self_check_in`, `device` or `import`.
    pub method: String,
    /// Whether the position was inside the geofence; `None` when unknown
    /// (no geofence is checked yet).
    pub within_geofence: Option<bool>,
    /// The flags the record carries when it is saved, e.g. `overlap`.
    pub flags: Vec<String>,
}

impl Event for AttendanceRecorded {}

/// A flag was added to an attendance record, once per record and flag type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendanceFlagged {
    /// The record.
    pub record: AttendanceRecordId,
    /// The employee.
    pub employee: EmployeeId,
    /// The day, in Asia/Dubai.
    pub date: NaiveDate,
    /// The flag type: `overlap`, `overtime_cap`, `midday_break`,
    /// `permit_invalid`, ...
    pub flag: String,
}

impl Event for AttendanceFlagged {}

/// Hours of one category, in minutes; signed in an adjustment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryMinutes {
    /// `normal`, `overtime`, `night`, `rest_day` or `public_holiday`.
    pub category: String,
    /// The minutes; negative for hours an adjustment takes away.
    pub minutes: i64,
}

/// A timesheet was approved (`approve`, `approve_on_behalf` or
/// `deem_approve`). Payroll and billing read it; for an adjustment `kind` is
/// `adjustment`, the totals are signed and `adjusts`, `origin` and `affects`
/// say what it corrects and where it flows (wl-attendance §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimesheetApproved {
    /// The timesheet, `attendance.timesheet:3`.
    pub timesheet: Subject,
    /// The employee whose hours these are.
    pub owner: EmployeeId,
    /// The first day.
    pub period_from: NaiveDate,
    /// The last day.
    pub period_to: NaiveDate,
    /// The context (project, deployment...), if any.
    pub context: Option<Subject>,
    /// Hours per category, summed over the entries.
    pub totals: Vec<CategoryMinutes>,
    /// `internal` or `external`.
    pub approver_kind: String,
    /// `approved`, `on_behalf` or `deemed`.
    pub approval_basis: String,
    /// `regular` or `adjustment`.
    pub kind: String,
    /// An adjustment: the locked timesheet it corrects.
    pub adjusts: Option<Subject>,
    /// An adjustment: `client_dispute`, `attendance_error`, `payroll_error`
    /// or `provisional_reconciliation`.
    pub origin: Option<String>,
    /// An adjustment: `billing`, `payroll` or `both`.
    pub affects: Option<String>,
}

impl Event for TimesheetApproved {}

/// The future a [`CostRateResolver`] answers with.
pub type LabourFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>>;

/// Says whether a context posts labour cost from approved hours and at what
/// rate (wl-attendance §4 *Labour cost*, §8 `CostRateResolver`). The app
/// provides it with [`provide_cost_rates`]: construction answers with the
/// trade's fully loaded rate, manpower and edtech do not post cost from hours.
/// Without one, [`cost_rates`] gives [`NoCostRates`]: nothing is posted.
pub trait CostRateResolver: Send + Sync + 'static {
    /// Whether approved hours of `context` post labour cost. The default is
    /// no, so that no cost is counted twice.
    fn posts_cost<'a>(
        &'a self,
        _state: &'a AppState,
        _context: Option<&'a Subject>,
    ) -> LabourFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The cost of one hour of `category` (`normal`, `overtime`, ...) for
    /// `employee` on `date` in the context, in fils of AED; `None` when no
    /// rate is known (nothing is posted for it).
    fn hourly_rate<'a>(
        &'a self,
        _state: &'a AppState,
        _employee: EmployeeId,
        _context: Option<&'a Subject>,
        _category: &'a str,
        _date: NaiveDate,
    ) -> LabourFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }
}

/// The resolver when none is provided: no context posts labour cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCostRates;

impl CostRateResolver for NoCostRates {}

/// Makes `rates` the app's [`CostRateResolver`].
pub fn provide_cost_rates(app: &mut Registry, rates: impl CostRateResolver) {
    let rates: std::sync::Arc<dyn CostRateResolver> = std::sync::Arc::new(rates);
    app.provide(rates);
}

/// The app's [`CostRateResolver`], or [`NoCostRates`] when none is provided.
pub fn cost_rates(state: &AppState) -> std::sync::Arc<dyn CostRateResolver> {
    match state.provided::<std::sync::Arc<dyn CostRateResolver>>() {
        Some(rates) => std::sync::Arc::clone(&rates),
        None => std::sync::Arc::new(NoCostRates),
    }
}

/// Says that a context (a project, a deployment...) stops accepting
/// attendance after a day (wl-attendance §4 `record`, 02 Construction §7.1
/// *Project* `terminate`). The app provides it with
/// [`provide_context_closure`]; `wl-attendance` asks it before saving a
/// record at a work location in the context. Without one, [`context_closure`]
/// gives [`NoContextClosure`]: every day is open.
pub trait ContextClosure: Send + Sync + 'static {
    /// The last day attendance is accepted in `context`, in Asia/Dubai; a
    /// record of a later day is refused. `None` when the context accepts
    /// attendance on any day. The default is none.
    fn last_working_day<'a>(
        &'a self,
        _state: &'a AppState,
        _context: &'a Subject,
    ) -> LabourFuture<'a, Option<NaiveDate>> {
        Box::pin(async { Ok(None) })
    }

    /// Why `employee` may not work in `context` on `date` (in Asia/Dubai), if
    /// they may not: a record of that day is refused with this text. 09
    /// Manpower says so for a worker whose deployment is suspended (an
    /// expired visa or licence: no attendance, no billing). The default is
    /// none: the context takes the employee.
    fn refuses<'a>(
        &'a self,
        _state: &'a AppState,
        _context: &'a Subject,
        _employee: EmployeeId,
        _date: NaiveDate,
    ) -> LabourFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

/// The closure when none is provided: no context closes.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoContextClosure;

impl ContextClosure for NoContextClosure {}

/// Makes `closure` the app's [`ContextClosure`].
pub fn provide_context_closure(app: &mut Registry, closure: impl ContextClosure) {
    let closure: std::sync::Arc<dyn ContextClosure> = std::sync::Arc::new(closure);
    app.provide(closure);
}

/// The app's [`ContextClosure`], or [`NoContextClosure`] when none is
/// provided.
pub fn context_closure(state: &AppState) -> std::sync::Arc<dyn ContextClosure> {
    match state.provided::<std::sync::Arc<dyn ContextClosure>>() {
        Some(closure) => std::sync::Arc::clone(&closure),
        None => std::sync::Arc::new(NoContextClosure),
    }
}

/// Lets a vertical send attendance to review. `wl-attendance` provides it
/// with [`provide_attendance_review`]; without it, [`attendance_review`]
/// gives [`NoAttendanceReview`] and nothing is flagged.
pub trait AttendanceReview: Send + Sync + 'static {
    /// Flags, for the reviewer, the attendance recorded after `after` at the
    /// work locations in `context`, with `note` (the records stay as they are
    /// and stay costed until accepted or adjusted). Returns the records
    /// flagged now; a record flagged before is not listed again. The default
    /// flags nothing.
    fn flag_after<'a>(
        &'a self,
        _state: &'a AppState,
        _context: &'a Subject,
        _after: NaiveDate,
        _note: &'a str,
    ) -> LabourFuture<'a, Vec<AttendanceRecordId>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Whether any attendance was recorded at a work location in `context`
    /// (a project that was cancelled may have none). The default is no.
    fn has_records<'a>(
        &'a self,
        _state: &'a AppState,
        _context: &'a Subject,
    ) -> LabourFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Rejects the attendance of `employee` from `from` on at the work
    /// locations in `contexts`, with `reason` (09 Manpower: a deployment
    /// entered in error, whose hours are neither billed nor paid). A record
    /// that is `recorded`, `flagged` or `accepted` (and not yet in a
    /// submitted timesheet) is rejected; one `locked`, or in a timesheet
    /// already submitted, cannot be, and is listed in
    /// [`WorkRejection::kept`] for an adjustment timesheet (`HRM.ACL.08`).
    /// The default rejects nothing.
    fn reject_work<'a>(
        &'a self,
        _state: &'a AppState,
        _contexts: &'a [Subject],
        _employee: EmployeeId,
        _from: NaiveDate,
        _reason: &'a str,
    ) -> LabourFuture<'a, WorkRejection> {
        Box::pin(async { Ok(WorkRejection::default()) })
    }
}

/// What [`AttendanceReview::reject_work`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkRejection {
    /// The records rejected now.
    pub rejected: Vec<AttendanceRecordId>,
    /// The records that stay because they are locked or in a submitted
    /// timesheet: an adjustment is needed.
    pub kept: Vec<AttendanceRecordId>,
}

/// The review when none is provided: nothing is flagged.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAttendanceReview;

impl AttendanceReview for NoAttendanceReview {}

/// Makes `review` the app's [`AttendanceReview`].
pub fn provide_attendance_review(app: &mut Registry, review: impl AttendanceReview) {
    let review: std::sync::Arc<dyn AttendanceReview> = std::sync::Arc::new(review);
    app.provide(review);
}

/// The app's [`AttendanceReview`], or [`NoAttendanceReview`] when none is
/// provided.
pub fn attendance_review(state: &AppState) -> std::sync::Arc<dyn AttendanceReview> {
    match state.provided::<std::sync::Arc<dyn AttendanceReview>>() {
        Some(review) => std::sync::Arc::clone(&review),
        None => std::sync::Arc::new(NoAttendanceReview),
    }
}

/// The days an employee was recorded as present: the last attended day an
/// absence case counts from (`CON.RUL.08`, `MPW.RUL.09`). `wl-attendance`
/// provides it with [`provide_attendance_history`]; without it,
/// [`attendance_history`] gives [`NoAttendanceHistory`]: nobody attended.
pub trait AttendanceHistory: Send + Sync + 'static {
    /// The days from `from` to `to` (both included, Asia/Dubai) on which
    /// `employee` has an attendance record that counts (any but a rejected
    /// one), oldest first, each day once. The default has none.
    fn attended_days<'a>(
        &'a self,
        _state: &'a AppState,
        _employee: EmployeeId,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> LabourFuture<'a, Vec<NaiveDate>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The attendance history when none is provided: nobody attended.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAttendanceHistory;

impl AttendanceHistory for NoAttendanceHistory {}

/// Makes `history` the app's [`AttendanceHistory`].
pub fn provide_attendance_history(app: &mut Registry, history: impl AttendanceHistory) {
    let history: std::sync::Arc<dyn AttendanceHistory> = std::sync::Arc::new(history);
    app.provide(history);
}

/// The app's [`AttendanceHistory`], or [`NoAttendanceHistory`] when none is
/// provided.
pub fn attendance_history(state: &AppState) -> std::sync::Arc<dyn AttendanceHistory> {
    match state.provided::<std::sync::Arc<dyn AttendanceHistory>>() {
        Some(history) => std::sync::Arc::clone(&history),
        None => std::sync::Arc::new(NoAttendanceHistory),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_a_subject_and_a_plain_number() {
        let id = AttendanceRecordId::from(7);
        assert_eq!(id.to_string(), "7");
        assert_eq!(id.subject(), Subject::new("attendance.record", 7));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "7");
    }

    #[test]
    fn the_events_round_trip_through_json() {
        let flagged = AttendanceFlagged {
            record: AttendanceRecordId(7),
            employee: EmployeeId(3),
            date: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            flag: "overlap".into(),
        };
        let json = renox::serde_json::to_string(&flagged).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<AttendanceFlagged>(&json).unwrap(),
            flagged
        );
    }

    #[test]
    fn the_approved_event_round_trips_through_json() {
        let approved = TimesheetApproved {
            timesheet: Subject::new("attendance.timesheet", 3),
            owner: EmployeeId(3),
            period_from: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            period_to: NaiveDate::from_ymd_opt(2026, 11, 8).unwrap(),
            context: None,
            totals: vec![CategoryMinutes {
                category: "normal".into(),
                minutes: -480,
            }],
            approver_kind: "internal".into(),
            approval_basis: "approved".into(),
            kind: "adjustment".into(),
            adjusts: Some(Subject::new("attendance.timesheet", 2)),
            origin: Some("payroll_error".into()),
            affects: Some("payroll".into()),
        };
        let json = renox::serde_json::to_string(&approved).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<TimesheetApproved>(&json).unwrap(),
            approved
        );
    }
}
