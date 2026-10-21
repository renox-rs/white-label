//! The workforce contract (wl-hr §6, BL-HRM-26): what other modules may know
//! about employees and their onboarding without depending on the `wl-hr`
//! crate.
//!
//! `wl-hr` provides [`Workforce`] with [`provide_workforce`] and emits
//! [`OnboardingStarted`], [`OnboardingCompleted`], [`EmployeeActivated`] and
//! [`EmployeeAssignmentChanged`]. The exit side of the employee lifecycle
//! (`cancel_onboarding`, `exit`, `record_death`) is built later; its events
//! [`EmployeeOnboardingCancelled`], [`EmployeeExited`] and
//! [`EmployeeDeceased`] are part of the contract already, because the
//! onboarding cancels itself on them. Without the `hr` module,
//! [`workforce`] gives [`NoWorkforce`]: nobody works anywhere. The SOP
//! acknowledgements (wl-documents `DOC.RUL.04`) also ask it for every employee
//! ([`Workforce::all_staff`]), the [`StaffProfile`] of one (role, scopes,
//! account, manager, leave) and the employee of a user.
//!
//! `wl-hr` also provides [`Qualifications`](crate::qualifications::Qualifications) (`qualification_valid`) and emits
//! [`QualificationExpiring`] and [`QualificationExpired`]. Its payroll runs
//! are announced with [`PayrollRunReturned`] (a run sent back to draft) and
//! [`PayrollLinesFailed`] (the bank rejected a SIF file, or lines of it) and
//! [`PayrollRunPaid`] (payment confirmed).
//!
//! An employee is referred to by [`EmployeeId`] or as a [`Subject`] of type
//! [`EMPLOYEE_SUBJECT_TYPE`]; a scope is a [`Subject`] too (a branch).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::documents::{AffectedGroup, BlockedGroup, PlannedRecord};
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of an employee, for [`Subject`] references.
pub const EMPLOYEE_SUBJECT_TYPE: &str = "hr.employee";

/// The subject type of an onboarding.
pub const ONBOARDING_SUBJECT_TYPE: &str = "hr.onboarding";

/// The subject type of one step of an onboarding.
pub const ONBOARDING_STEP_SUBJECT_TYPE: &str = "hr.onboarding_step";

/// The id of an employee.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct EmployeeId(pub i64);

impl EmployeeId {
    /// The employee as a [`Subject`]: `hr.employee:7`.
    pub fn subject(self) -> Subject {
        Subject::new(EMPLOYEE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for EmployeeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for EmployeeId {
    fn from(id: i64) -> Self {
        EmployeeId(id)
    }
}

/// The id of an onboarding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OnboardingId(pub i64);

impl OnboardingId {
    /// The onboarding as a [`Subject`]: `hr.onboarding:3`.
    pub fn subject(self) -> Subject {
        Subject::new(ONBOARDING_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for OnboardingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for OnboardingId {
    fn from(id: i64) -> Self {
        OnboardingId(id)
    }
}

/// The subject type of a payroll run, for [`Subject`] references.
pub const PAYROLL_RUN_SUBJECT_TYPE: &str = "hr.payroll_run";

/// The id of a payroll run.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PayrollRunId(pub i64);

impl PayrollRunId {
    /// The run as a [`Subject`]: `hr.payroll_run:4`.
    pub fn subject(self) -> Subject {
        Subject::new(PAYROLL_RUN_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for PayrollRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for PayrollRunId {
    fn from(id: i64) -> Self {
        PayrollRunId(id)
    }
}

/// An approver sent a payroll run back to `draft` with a reason and comments
/// on the lines to fix (`return`, wl-hr §4 *Payroll run*): the approval is
/// cleared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollRunReturned {
    /// The run.
    pub run: PayrollRunId,
    /// Its period, e.g. `2026-11`.
    pub period: String,
    /// Why it was returned.
    pub reason: String,
}

impl Event for PayrollRunReturned {}

/// A payroll line the bank failed, with the bank's reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedPayrollLine {
    /// The payroll line's id.
    pub line: i64,
    /// The employee it pays.
    pub employee: EmployeeId,
    /// Why the bank failed it (the rejection reason of a whole file, or the
    /// reason from the result file).
    pub reason: String,
}

/// The bank or exchange house rejected the whole SIF file
/// (`record_rejection`), or failed some lines of it (`record_partial`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollLinesFailed {
    /// The run.
    pub run: PayrollRunId,
    /// Its period, e.g. `2026-11`.
    pub period: String,
    /// The failed lines with their reasons.
    pub lines: Vec<FailedPayrollLine>,
}

impl Event for PayrollLinesFailed {}

/// Payment of a payroll run was confirmed (`confirm_paid`): the payslips are
/// published. Held lines wait for a later run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollRunPaid {
    /// The run.
    pub run: PayrollRunId,
    /// Its period, e.g. `2026-11`.
    pub period: String,
    /// The sum of the nets of the lines paid, AED fils.
    pub total: i64,
    /// The employees paid.
    pub employees: Vec<EmployeeId>,
}

impl Event for PayrollRunPaid {}

/// The subject type of a pay account, for [`Subject`] references.
pub const PAY_ACCOUNT_SUBJECT_TYPE: &str = "hr.pay_account";

/// A new pay account became `active` (`activate`, or `waive_cooling` once an
/// MD approved it): payroll pays the employee into it from the next run
/// submitted. The account number itself is never in the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayAccountChanged {
    /// The employee.
    pub employee: EmployeeId,
    /// The last 4 digits of the IBAN or card number.
    pub last4: String,
    /// The user who approved the account (at `approve`).
    pub approver: Option<i64>,
}

impl Event for PayAccountChanged {}

/// An employee became `active` (`activate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeActivated {
    /// The employee.
    pub employee: EmployeeId,
    /// The employee's first scope, if any.
    pub scope: Option<Subject>,
    /// The hire date.
    pub hire_date: NaiveDate,
}

impl Event for EmployeeActivated {}

/// An onboarding was cancelled before the employee started
/// (`cancel_onboarding`, approved under `HRM.ACL.07`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeOnboardingCancelled {
    /// The employee.
    pub employee: EmployeeId,
    /// Why.
    pub reason: String,
}

impl Event for EmployeeOnboardingCancelled {}

/// An employee exited (`exit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeExited {
    /// The employee.
    pub employee: EmployeeId,
    /// The exit date.
    pub exit_date: NaiveDate,
    /// `standard`, `absconding` or `death`.
    pub route: String,
}

impl Event for EmployeeExited {}

/// An employee died in service (`record_death`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeDeceased {
    /// The employee.
    pub employee: EmployeeId,
    /// The date of death.
    pub date: NaiveDate,
}

impl Event for EmployeeDeceased {}

/// A vertical opened an absence case for an employee (`MPW.RUL.09`,
/// `CON.RUL.08`): N consecutive days without attendance or approved leave.
/// `wl-hr` keeps it, because `report_absconding` needs an open case and
/// counts the days from `last_present_on`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsenceCaseOpened {
    /// The employee.
    pub employee: EmployeeId,
    /// The case, e.g. `ticketing.ticket:12`.
    pub case: Subject,
    /// The day the case was opened.
    pub opened_on: NaiveDate,
    /// The last day with attendance or approved leave (day 0 of the
    /// absence), as the vertical counted it from the attendance records.
    pub last_present_on: NaiveDate,
}

impl Event for AbsenceCaseOpened {}

/// A vertical closed the absence case of an employee, e.g. the worker
/// returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbsenceCaseClosed {
    /// The employee.
    pub employee: EmployeeId,
    /// The case.
    pub case: Subject,
    /// The day it was closed.
    pub closed_on: NaiveDate,
    /// Why.
    pub reason: String,
}

impl Event for AbsenceCaseClosed {}

/// An employee was reported as absconded (`report_absconding`, approved under
/// `HRM.ACL.07`): the apps end the deployments, the portal and attendance are
/// off, the open pay is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeAbsconded {
    /// The employee.
    pub employee: EmployeeId,
    /// The MoHRE absence report number.
    pub report_number: String,
    /// The MoHRE absence report date.
    pub report_date: NaiveDate,
}

impl Event for EmployeeAbsconded {}

/// A scope assignment or the job role of an employee was added, changed or
/// ended. It does not start an onboarding by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmployeeAssignmentChanged {
    /// The employee.
    pub employee: EmployeeId,
    /// The job role after the change.
    pub job_role: String,
    /// The scope that was added, changed or ended, if the change is one.
    pub scope: Option<Subject>,
    /// What it was before (a job role or a scope), if anything.
    pub from: Option<String>,
    /// What it is now, if anything.
    pub to: Option<String>,
}

impl Event for EmployeeAssignmentChanged {}

/// An onboarding started (`start`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingStarted {
    /// The onboarding.
    pub onboarding: OnboardingId,
    /// The employee.
    pub employee: EmployeeId,
    /// The employee's job role.
    pub job_role: String,
    /// The employee's first scope, if any.
    pub scope: Option<Subject>,
    /// `hire` or `manual`.
    pub trigger: String,
}

impl Event for OnboardingStarted {}

/// An onboarding completed (`complete`): every required step is done or
/// waived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingCompleted {
    /// The onboarding.
    pub onboarding: OnboardingId,
    /// The employee.
    pub employee: EmployeeId,
    /// The employee's job role.
    pub job_role: String,
    /// The employee's first scope, if any.
    pub scope: Option<Subject>,
    /// `hire` or `manual`.
    pub trigger: String,
}

impl Event for OnboardingCompleted {}

/// The future the [`Workforce`] reads return.
pub type WorkforceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What other modules ask of the workforce. `wl-hr` provides it; without that
/// module, [`workforce`] gives [`NoWorkforce`].
pub trait Workforce: Send + Sync + 'static {
    /// The employees (`onboarding`, `active`, `on_leave`, `notice`) with the
    /// job role or trade `role` in `scope`; not those who are `absconded`,
    /// `deceased`, `exited` or whose onboarding was cancelled. The default
    /// has none.
    fn employees_in<'a>(
        &'a self,
        _db: &'a Db,
        _role: &'a str,
        _scope: &'a Subject,
    ) -> WorkforceFuture<'a, Vec<EmployeeId>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Every employee [`employees_in`](Workforce::employees_in) would list,
    /// whatever the role. The default has none.
    fn all_staff<'a>(&'a self, _db: &'a Db) -> WorkforceFuture<'a, Vec<EmployeeId>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The profile of `employee`, or `None` for an unknown employee. The
    /// default knows nobody.
    fn staff<'a>(
        &'a self,
        _state: &'a AppState,
        _employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async { Ok(None) })
    }

    /// Whether the account of user `user_id` belongs to an employee whose
    /// portal access is switched off: one reported as absconded (`HRM.RUL.14`).
    /// The default switches nobody off.
    fn access_disabled<'a>(&'a self, _db: &'a Db, _user_id: i64) -> WorkforceFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The employee whose account is the user `user_id`, if any. The default
    /// knows nobody.
    fn employee_of_user<'a>(
        &'a self,
        _db: &'a Db,
        _user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async { Ok(None) })
    }
}

/// What the SOP acknowledgements (wl-documents `DOC.RUL.04`) need to know
/// about a member of staff, without depending on the `wl-hr` crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaffProfile {
    /// The employee.
    pub employee: EmployeeId,
    /// The person (a party): the contact points of an SMS are theirs.
    pub party: PartyId,
    /// The account the employee signs in with; `None` when they have none
    /// and everything is done in person.
    pub user_id: Option<i64>,
    /// The job role or trade key.
    pub role: String,
    /// The scopes the employee works in.
    pub scopes: Vec<Subject>,
    /// The employing legal entity, if recorded.
    pub legal_entity_id: Option<i64>,
    /// Whether [`Workforce::employees_in`] lists the employee: `onboarding`,
    /// `active`, `on_leave` or `notice`, not `absconded` or gone.
    pub listed: bool,
    /// Whether the employee is `on_leave` now.
    pub on_leave: bool,
    /// The day the employee is back at work, when `on_leave` and the leave
    /// is recorded: the day after the last day of the approved leave.
    pub returns_on: Option<NaiveDate>,
    /// The users who are the employee's manager (wl-hr §3 *The employee's
    /// manager*): the holders of the manager role in the employee's scopes,
    /// HR when there are none.
    pub managers: Vec<i64>,
}

/// The id of a qualification (a licence, permit or certificate of an
/// employee).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct QualificationId(pub i64);

impl fmt::Display for QualificationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One record of an employee's work that a qualification blocks: an open or
/// planned booking, deployment or run, as `wl-documents` found it
/// ([`BlockedGroup`], [`AffectedGroup`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedWork {
    /// The guarded transition, `<entity>.<transition>`.
    pub guarded: String,
    /// The record.
    pub record: Subject,
    /// The date it is planned for (Asia/Dubai calendar date).
    pub date: NaiveDate,
    /// What to show to the staff.
    pub label: String,
    /// Whether it is already running (open) rather than planned.
    pub open: bool,
}

impl AffectedWork {
    /// `record`, found under the guarded transition `guarded`.
    pub fn of(guarded: &str, record: &PlannedRecord, open: bool) -> Self {
        AffectedWork {
            guarded: guarded.to_owned(),
            record: record.record.clone(),
            date: record.date,
            label: record.label.clone(),
            open,
        }
    }

    /// The planned records of `groups`, in order, unchanged
    /// (`DocumentExpiring::affected`).
    pub fn planned_in(groups: &[AffectedGroup]) -> Vec<AffectedWork> {
        groups
            .iter()
            .flat_map(|group| {
                group
                    .records
                    .iter()
                    .map(|record| AffectedWork::of(&group.guarded, record, false))
            })
            .collect()
    }

    /// The open and then the planned records of each of `groups`
    /// (`DocumentExpired::blocked`).
    pub fn blocked_in(groups: &[BlockedGroup]) -> Vec<AffectedWork> {
        groups
            .iter()
            .flat_map(|group| {
                let open = group
                    .open
                    .iter()
                    .map(|record| AffectedWork::of(&group.guarded, record, true));
                let planned = group
                    .planned
                    .iter()
                    .map(|record| AffectedWork::of(&group.guarded, record, false));
                open.chain(planned)
            })
            .collect()
    }
}

/// A qualification of an employee is about to expire (`HRM.RUL.10`, from
/// `DocumentExpiring`). It carries the planned work after the expiry date, as
/// the document event listed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationExpiring {
    /// The employee.
    pub employee: EmployeeId,
    /// The qualification record, when the employee has one of the kind.
    pub qualification: Option<QualificationId>,
    /// The document kind.
    pub kind: String,
    /// The scopes (branches) the qualification covers; empty for any.
    pub scope: Vec<Subject>,
    /// The expiry date; the qualification is valid through it.
    pub expires_on: NaiveDate,
    /// Expiry date minus today.
    pub days_left: i64,
    /// The planned work after the expiry date.
    pub affected: Vec<AffectedWork>,
}

impl Event for QualificationExpiring {}

/// A qualification of an employee expired (`HRM.RUL.10`, from
/// `DocumentExpired`): `qualification_valid` answers *no* from now on. It
/// carries the employee's open and planned work that the expiry blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationExpired {
    /// The employee.
    pub employee: EmployeeId,
    /// The qualification record, when the employee has one of the kind.
    pub qualification: Option<QualificationId>,
    /// The document kind.
    pub kind: String,
    /// The scopes (branches) the qualification covered; empty for any.
    pub scope: Vec<Subject>,
    /// The roles the qualification covered; empty for any.
    pub roles: Vec<String>,
    /// The expiry date; the qualification was valid through it.
    pub expires_on: NaiveDate,
    /// The open and planned work the expiry blocks.
    pub affected: Vec<AffectedWork>,
}

impl Event for QualificationExpired {}

/// The approved leave of an employee (wl-hr §4 *Leave request*): what the
/// verticals need to tell an absence from a leave (`CON.RUL.08`). `wl-hr`
/// provides it; without it, [`leave_calendar`] gives [`NoLeaveCalendar`]:
/// nobody is on leave.
pub trait LeaveCalendar: Send + Sync + 'static {
    /// The periods of approved (or completed) leave of `employee` that touch
    /// the days `from` to `to` (both included), as `(first day, last day)`
    /// pairs, oldest first. The default has none.
    fn approved_leave<'a>(
        &'a self,
        _db: &'a Db,
        _employee: EmployeeId,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> WorkforceFuture<'a, Vec<(NaiveDate, NaiveDate)>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The leave calendar when the `hr` module is off: nobody is on leave.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLeaveCalendar;

impl LeaveCalendar for NoLeaveCalendar {}

/// Makes `calendar` the app's [`LeaveCalendar`]; `wl-hr` calls it in
/// `register`.
pub fn provide_leave_calendar(app: &mut Registry, calendar: impl LeaveCalendar) {
    let calendar: Arc<dyn LeaveCalendar> = Arc::new(calendar);
    app.provide(calendar);
}

/// The app's [`LeaveCalendar`], or [`NoLeaveCalendar`] when none is provided.
pub fn leave_calendar(state: &AppState) -> Arc<dyn LeaveCalendar> {
    match state.provided::<Arc<dyn LeaveCalendar>>() {
        Some(calendar) => Arc::clone(&calendar),
        None => Arc::new(NoLeaveCalendar),
    }
}

/// The workforce when the `hr` module is off: nobody works anywhere.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWorkforce;

impl Workforce for NoWorkforce {}

/// Makes `workforce` the app's [`Workforce`]; `wl-hr` calls it in `register`.
pub fn provide_workforce(app: &mut Registry, workforce: impl Workforce) {
    let workforce: Arc<dyn Workforce> = Arc::new(workforce);
    app.provide(workforce);
}

/// The app's [`Workforce`], or [`NoWorkforce`] when none is provided.
pub fn workforce(state: &AppState) -> Arc<dyn Workforce> {
    match state.provided::<Arc<dyn Workforce>>() {
        Some(workforce) => Arc::clone(&workforce),
        None => Arc::new(NoWorkforce),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_employee_is_a_subject_and_a_plain_number() {
        let id = EmployeeId::from(7);
        assert_eq!(id.to_string(), "7");
        assert_eq!(id.subject(), Subject::new("hr.employee", 7));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "7");
        assert_eq!(OnboardingId(3).subject(), Subject::new("hr.onboarding", 3));
    }

    #[test]
    fn a_staff_profile_round_trips_through_json() {
        let profile = StaffProfile {
            employee: EmployeeId(7),
            party: PartyId(50),
            user_id: None,
            role: "site_worker".into(),
            scopes: vec![Subject::new("property.site", 2)],
            legal_entity_id: Some(1),
            listed: true,
            on_leave: true,
            returns_on: NaiveDate::from_ymd_opt(2026, 12, 1),
            managers: vec![4, 5],
        };
        let json = renox::serde_json::to_value(&profile).unwrap();
        assert_eq!(json["employee"], 7);
        assert_eq!(json["party"], 50);
        assert!(json["user_id"].is_null());
        assert_eq!(
            renox::serde_json::from_value::<StaffProfile>(json).unwrap(),
            profile
        );
    }

    #[test]
    fn the_events_round_trip_through_json() {
        let started = OnboardingStarted {
            onboarding: OnboardingId(3),
            employee: EmployeeId(7),
            job_role: "driver".into(),
            scope: Some(Subject::new("property.site", 2)),
            trigger: "hire".into(),
        };
        let json = renox::serde_json::to_string(&started).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<OnboardingStarted>(&json).unwrap(),
            started
        );
        let changed = EmployeeAssignmentChanged {
            employee: EmployeeId(7),
            job_role: "driver".into(),
            scope: None,
            from: Some("cleaner".into()),
            to: Some("driver".into()),
        };
        let json = renox::serde_json::to_value(&changed).unwrap();
        assert_eq!(json["employee"], 7);
        assert!(json["scope"].is_null());
    }
}
