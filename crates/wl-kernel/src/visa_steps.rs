//! The steps of the visa cases that are still open (wl-hr §8 `VisaStepTemplate`:
//! each step has a target number of days from the previous mandatory step or
//! the case start). The vertical's rule that tells people about a late step
//! (09: `MPW.RUL.14`) reads them through [`VisaSteps`] without depending on
//! `wl-hr`, which provides it ([`provide_visa_steps`]). Without it
//! [`visa_steps`] gives [`NoVisaSteps`]: no step is late.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;
use crate::workforce::EmployeeId;

/// A step of a live visa case (`open`, `in_progress` or `on_hold`) that is not
/// done, has a target and has started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenVisaStep {
    /// The step's id (one notice per step).
    pub step: i64,
    /// The case (`hr.visa_case:7`).
    pub case: Subject,
    /// The employee the case is for.
    pub employee: EmployeeId,
    /// `new`, `renewal` or `cancellation`.
    pub case_type: String,
    /// The step's type (`medical_fitness`, `work_permit`...).
    pub step_type: String,
    /// The day the step started: the day the previous mandatory step was done,
    /// or the day the case was created for the first one.
    pub started_on: NaiveDate,
    /// The target number of days.
    pub target_days: i64,
}

/// The future a [`VisaSteps`] method returns.
pub type VisaStepsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The open steps of the visa cases. `wl-hr` provides it.
pub trait VisaSteps: Send + Sync + 'static {
    /// Every step of a live case that is not done, has a target and has started
    /// (the previous mandatory step is done), oldest first. The default has
    /// none.
    fn open_steps<'a>(&'a self, _state: &'a AppState) -> VisaStepsFuture<'a, Vec<OpenVisaStep>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Whether `step` is still not done on a live case (a case that was
    /// cancelled or rejected meanwhile has none). The default says no.
    fn is_open<'a>(&'a self, _state: &'a AppState, _step: i64) -> VisaStepsFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The steps when `wl-hr` is off: none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVisaSteps;

impl VisaSteps for NoVisaSteps {}

/// Makes `steps` the app's [`VisaSteps`]; `wl-hr` calls it in `register`.
pub fn provide_visa_steps(app: &mut Registry, steps: impl VisaSteps) {
    let steps: Arc<dyn VisaSteps> = Arc::new(steps);
    app.provide(steps);
}

/// The app's [`VisaSteps`], or [`NoVisaSteps`] when none is provided.
pub fn visa_steps(state: &AppState) -> Arc<dyn VisaSteps> {
    match state.provided::<Arc<dyn VisaSteps>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoVisaSteps),
    }
}
