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
//! [`workforce`] gives [`NoWorkforce`]: nobody works anywhere.
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
