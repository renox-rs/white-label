//! The driver and vehicle document guard (wl-delivery §4.1 `assign`, §4.2
//! `dispatch`, issue #157): a delivery is not put on a run, and a run is not
//! dispatched, while its driver's licence or rider permit, or its vehicle's
//! registration (Mulkiya) or insurance, is not valid.
//!
//! Which kinds block is never decided here: it is the document kinds'
//! `blocks` (wl-documents §5.6; 03: driving licence, rider permit, Mulkiya
//! and insurance block `delivery.assign` and `run.dispatch`, legal blocks
//! with the override mode `never`). This module registers the
//! [`BlockSubjects`] provider that maps a **run** to the subjects whose
//! documents must be valid, for both guarded transitions:
//!
//! | Guarded | Record | Subjects |
//! |---|---|---|
//! | `delivery.assign` | the run the delivery is put on | its driver (an employee, `hr.employee`; the user's account when the `hr` module is off) and its vehicle (an asset, `inventory.asset`; none for an on-foot rider) |
//! | `run.dispatch` | the run | the same |
//!
//! The document kind says which subject type it attaches to
//! ([`DocumentGuard::kind_subject_types`]): a licence is checked on the
//! driver, an insurance on the vehicle. `qualification_valid` of `wl-hr` is
//! the wrapper of `document_valid` on the employee; this module asks the
//! document guard for the employee subject directly, which is what the
//! wrapper does.
//!
//! [`run`] is the call both transitions go through: the check with the
//! business date (`assign`: the run's date; `dispatch`: today), then the
//! transition. A refusal is a 422 with one line per failing document (the
//! document, its expiry date and who can renew it come from wl-documents)
//! and, only for a kind that allows an override, the link that requests it.
//! Without `wl-documents` the transition just runs.

use std::future::Future;
use std::sync::{Mutex, PoisonError};

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::Subject;
use wl_kernel::documents::{
    self, BlockEntry, BlockRefusal, BlockSubjects, DocumentFuture, guarded_transition_as,
    register_block_subjects,
};
use wl_kernel::workforce::workforce;

use crate::run::{DRIVER_USER_SUBJECT_TYPE, Run};

/// The guarded transition `assign` of a delivery.
pub const ASSIGN: &str = "delivery.assign";

/// The guarded transition `dispatch` of a run.
pub const DISPATCH: &str = "run.dispatch";

/// The document subject of the driver of `run`: the employee of the user
/// when `wl-hr` knows one, else the user's account.
pub async fn driver_subject(state: &AppState, run: &Run) -> Result<Subject> {
    Ok(workforce(state)
        .employee_of_user(&state.db, run.driver_user_id)
        .await?
        .map_or_else(
            || Subject::new(DRIVER_USER_SUBJECT_TYPE, run.driver_user_id),
            |employee| employee.subject(),
        ))
}

/// Maps a run to its driver and vehicle.
struct RunSubjects;

impl BlockSubjects for RunSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(run) = Run::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let mut candidates = vec![driver_subject(state, &run).await?];
            candidates.extend(run.vehicle_subject());
            let types = documents::guard(state)
                .kind_subject_types(&state.db, &entry.kind)
                .await?;
            if types.is_empty() {
                // The kind is unknown to the guard: every subject is checked.
                return Ok(candidates);
            }
            Ok(candidates
                .into_iter()
                .filter(|subject| types.contains(&subject.subject_type))
                .collect())
        })
    }
}

/// Registers the provider for both guarded transitions.
pub(crate) fn register() {
    register_block_subjects(ASSIGN, RunSubjects);
    register_block_subjects(DISPATCH, RunSubjects);
}

/// The message of a refusal: one line per failing document, then, for each
/// kind that allows an override, the link that requests it. A legal block
/// (`never`) offers none.
fn refusal_text(refusal: &BlockRefusal) -> String {
    let mut text = refusal.message();
    for link in refusal.override_links() {
        text.push_str("\nRequest an override: ");
        text.push_str(&link);
    }
    text
}

/// Runs `transition` as the guarded transition `guarded` of the run
/// `run_id`, checking the driver's and vehicle's documents on the business
/// date `on` first. A refusal answers 422 with [`refusal_text`] and the
/// transition does not run; a transition that fails is returned as it is.
///
/// # Errors
///
/// `Guard` (422) with the refusal, or the transition's own error.
pub(crate) async fn run<F, Fut>(
    state: &AppState,
    guarded: &str,
    run_id: i64,
    on: NaiveDate,
    actor: Option<i64>,
    transition: F,
) -> std::result::Result<(), TransitionError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = std::result::Result<(), TransitionError>>,
{
    // The structured check first: the refusal keeps its override links.
    let checked = documents::guard(state)
        .check_as(state, guarded, run_id, on, actor)
        .await
        .map_err(TransitionError::Failed)?;
    if let Err(refusal) = checked {
        return Err(TransitionError::Guard(refusal_text(&refusal)));
    }
    let failed: Mutex<Option<TransitionError>> = Mutex::new(None);
    let result = guarded_transition_as(state, guarded, run_id, on, actor, || async {
        match transition().await {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                *failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))
            }
        }
    })
    .await;
    if let Some(error) = failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    result.map_err(TransitionError::Failed)
}
