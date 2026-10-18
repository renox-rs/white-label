//! Decisions that say yes (wl-kernel §4.1 `approve_step`, `approve`,
//! `fail`).
//!
//! An approver of the current step (a holder of one of its roles, or of its
//! permission, in the subject's scope; with SoD on, never the requester nor
//! whoever approved another step) approves it. [`approve_in`] does it inside
//! one transaction:
//!
//! 1. the request row is loaded locked; one that is no longer `pending`, or
//!    whose step moved on since the caller checked, is refused ("this
//!    request was already decided");
//! 2. the approver's decision is stored;
//! 3. on a step that is not the last, the request runs `approve_step` and
//!    waits for the next step;
//! 4. on the last step, the subject runs `<transition>_approved` as the
//!    system, with the payload snapshot as its input, inside a savepoint.
//!    When it runs, the request runs `approve`. When the action's guards
//!    refuse (or the subject is no longer in a state the action leaves), the
//!    savepoint rolls back and the request ends with `fail` through the end
//!    path ([`super::end::end_in`]), with the reason "approval failed:
//!    <guard message>": the subject goes back like on `reject`.
//!
//! Before step 4 the final re-check hooks of the subject's type run
//! ([`register_final_check`]): a hook that answers with a message ends the
//! request with `fail` the same way, and `<transition>_approved` does not run.
//!
//! Any other failure rolls the whole decision back. Once the transaction has
//! committed, [`Approval::announce`] emits the events and the notices.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, TransitionError};

use super::end::{self, End, Ended};
use super::events::{ApprovalDecided, ApprovalEnd};
use super::notify;
use super::request::{ApprovalDecision, ApprovalRequest, DecisionKind, transitions};
use crate::machines::{self, Ran, Run, SubjectMachine};
use crate::voids;

/// A hook that checks, at the final approval of a request and before
/// `<transition>_approved` runs, whether the action may still go ahead: the
/// general mechanism for rules the transition's own guards cannot read
/// (they have no database). It runs inside the decision's transaction and
/// answers `Ok(None)` to let the action run, or `Ok(Some(message))` to fail
/// the request with the reason "approval failed: <message>" (the subject
/// goes back like on `reject`). An `Err` rolls the whole decision back.
pub type FinalCheck = Arc<
    dyn for<'a> Fn(
            &'a mut Transaction,
            &'a ApprovalRequest,
        ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>
        + Send
        + Sync,
>;

static FINAL_CHECKS: RwLock<BTreeMap<String, FinalCheck>> = RwLock::new(BTreeMap::new());

/// Registers the final re-check of requests about `subject_type`, replacing
/// the one registered before. Process-wide, like the default policies.
pub fn register_final_check<F>(subject_type: impl Into<String>, check: F)
where
    F: for<'a> Fn(
            &'a mut Transaction,
            &'a ApprovalRequest,
        ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>>
        + Send
        + Sync
        + 'static,
{
    FINAL_CHECKS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(subject_type.into(), Arc::new(check));
}

fn final_check(subject_type: &str) -> Option<FinalCheck> {
    FINAL_CHECKS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(subject_type)
        .cloned()
}

/// The action's refusal at final approval, carried out of the savepoint.
#[derive(Debug)]
struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Whether the engine's refusal of `<transition>_approved` means the action
/// can no longer run (the request fails), with the message to give; `None`
/// for a failure that rolls the whole decision back.
fn refusal_message(error: &TransitionError) -> Option<String> {
    match error {
        TransitionError::Guard(message) => Some(message.clone()),
        TransitionError::NotFrom(_) => Some(error.to_string()),
        _ => None,
    }
}

/// The error a refusal travels in through the savepoint.
fn into_savepoint_error(error: TransitionError) -> Error {
    match refusal_message(&error) {
        Some(message) => Error::Internal(renox::anyhow::Error::new(Refused(message))),
        None => error.into_error(),
    }
}

/// The refusal's message, when `error` carries one; the error otherwise.
fn refusal(error: Error) -> std::result::Result<String, Error> {
    match error {
        Error::Internal(inner) => match inner.downcast::<Refused>() {
            Ok(Refused(message)) => Ok(message),
            Err(inner) => Err(Error::Internal(inner)),
        },
        other => Err(other),
    }
}

/// The history rows' metadata of a yes.
fn meta(request: &ApprovalRequest, approver_id: i64, approvers: &[i64]) -> Value {
    json!({
        "approval_request": request.id,
        "decision": ApprovalEnd::Approved,
        "step": request.current_step,
        "actor_id": approver_id,
        "requester_id": request.requester_id,
        "approvers": approvers,
    })
}

/// What an approval did, in a transaction that has not committed yet. Call
/// [`Approval::announce`] once it has.
#[must_use = "announce it after the transaction commits"]
pub struct Approval {
    /// The request, as it is after the decision.
    pub request: ApprovalRequest,
    /// The users who approved a step of the request, in order.
    pub approvers: Vec<i64>,
    kind: Kind,
}

enum Kind {
    /// A step that was not the last: the request waits for the next one.
    Step {
        request_run: Outcome<ApprovalRequest>,
    },
    /// The last step: the action ran.
    Approved {
        request_run: Outcome<ApprovalRequest>,
        subject_run: Option<Ran>,
    },
    /// The last step, but the action's guards refused: the request failed.
    Failed(Box<Ended>),
}

impl fmt::Debug for Approval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Approval")
            .field("request", &self.request.id)
            .field("approvers", &self.approvers)
            .field("end", &self.end())
            .finish()
    }
}

impl Approval {
    /// How the request ended: `Approved`, `Failed`, or `None` when it waits
    /// for its next step.
    pub fn end(&self) -> Option<ApprovalEnd> {
        match &self.kind {
            Kind::Step { .. } => None,
            Kind::Approved { .. } => Some(ApprovalEnd::Approved),
            Kind::Failed(_) => Some(ApprovalEnd::Failed),
        }
    }

    /// After the commit: emits the engine's `Transitioned` events, then, for
    /// an end, `ApprovalDecided`; tells the next step's approvers that the
    /// request waits for them, the requester that it was approved, or (on
    /// `fail`) the requester and the approver why it failed. Returns the
    /// event; `None` when the request waits for its next step.
    pub async fn announce(self, state: &AppState) -> Result<Option<ApprovalDecided>> {
        match self.kind {
            Kind::Step { request_run } => {
                request_run.emit(state).await?;
                let next = notify::step_approvers(&state.db, &self.request).await?;
                notify::requested(state, &self.request, &next).await?;
                Ok(None)
            }
            Kind::Approved {
                request_run,
                subject_run,
            } => {
                let decided = ApprovalDecided {
                    request: self.request.id,
                    decision: ApprovalEnd::Approved,
                    approvers: self.approvers,
                    subject: self.request.subject(),
                    state_returned_to: None,
                };
                request_run.emit(state).await?;
                if let Some(run) = subject_run {
                    run.emit(state).await?;
                    // A `void` that ran also emits `RecordVoided`.
                    voids::approval::announce_approved(state, &self.request).await?;
                }
                state.emit(decided.clone()).await?;
                notify::approved(state, &self.request).await?;
                Ok(Some(decided))
            }
            Kind::Failed(ended) => ended.announce(state).await.map(Some),
        }
    }
}

/// Runs `<transition>_approved` on the subject of `request` inside a
/// savepoint: `Ok(Ok(run))` when it ran, `Ok(Err(message))` when the action
/// refused (nothing of it stays), `Err` for any other failure.
async fn run_approved(
    tx: &mut Transaction,
    machine: Arc<SubjectMachine>,
    request: &ApprovalRequest,
    meta: Value,
) -> Result<std::result::Result<Ran, String>> {
    let subject_id = request.subject_id;
    let name = machines::approved(&request.transition);
    let mut run = Run::new().input(request.payload.0.clone()).meta(meta);
    // A reason kept in the payload snapshot (a void's) is the run's reason.
    if let Some(reason) = request.payload.0.get("reason").and_then(Value::as_str) {
        run = run.reason(reason);
    }
    let attempt = tx
        .savepoint(move |tx| {
            Box::pin(async move {
                machine
                    .run_in(tx, subject_id, &name, run)
                    .await
                    .map_err(into_savepoint_error)
            })
        })
        .await;
    match attempt {
        Ok(ran) => Ok(Ok(ran)),
        Err(error) => refusal(error).map(Err),
    }
}

/// `approver_id` approves step `step` of request `id`, inside `tx`, without
/// committing (see the module's steps); `comment` is optional (blank for
/// none). The caller has checked that the approver may decide the step
/// ([`approve`] does). Refused with 422 when the request is no longer
/// pending or its step is no longer `step`.
pub async fn approve_in(
    tx: &mut Transaction,
    id: i64,
    approver_id: i64,
    step: i64,
    comment: &str,
) -> Result<Approval> {
    let request = end::lock(tx, id).await?;
    if !request.is_pending() || request.current_step != step {
        return Err(end::already_decided());
    }
    let comment = comment.trim();
    ApprovalDecision::create(
        &mut *tx,
        ApprovalDecision {
            request_id: request.id,
            step: request.current_step,
            approver_id,
            decision: DecisionKind::Approve,
            comment: (!comment.is_empty()).then(|| comment.to_owned()),
            ..Default::default()
        },
    )
    .await?;
    let approvers: Vec<i64> = request
        .decisions(&mut *tx)
        .await?
        .into_iter()
        .map(|decision| decision.approver_id)
        .collect();
    let meta = meta(&request, approver_id, &approvers);

    if !request.on_last_step() {
        let request_run = request
            .transition(transitions::APPROVE_STEP)
            .by_system()
            .meta(meta)
            .run_in(tx)
            .await
            .map_err(end::refused)?;
        // A second approval of the same step, racing this one, finds the
        // step moved on and is refused.
        let moved = ApprovalRequest::where_eq("id", request.id)
            .where_eq("current_step", request.current_step)
            .update(&mut *tx, &[("current_step", &(request.current_step + 1))])
            .await?;
        if moved == 0 {
            return Err(end::already_decided());
        }
        let request = ApprovalRequest::find(&mut *tx, request.id)
            .await?
            .ok_or(Error::NotFound)?;
        return Ok(Approval {
            request,
            approvers,
            kind: Kind::Step { request_run },
        });
    }

    if let Some(check) = final_check(&request.subject_type)
        && let Some(message) = check(&mut *tx, &request).await?
    {
        let ended = end::end_in(tx, request.id, End::fail(Some(approver_id), &message)).await?;
        return Ok(Approval {
            request: ended.request.clone(),
            approvers: ended.approvers.clone(),
            kind: Kind::Failed(Box::new(ended)),
        });
    }
    let mut subject_run = None;
    if let Some(machine) = machines::get(&request.subject_type) {
        match run_approved(tx, machine, &request, meta.clone()).await? {
            Ok(ran) => subject_run = Some(ran),
            Err(message) => {
                let ended =
                    end::end_in(tx, request.id, End::fail(Some(approver_id), &message)).await?;
                return Ok(Approval {
                    request: ended.request.clone(),
                    approvers: ended.approvers.clone(),
                    kind: Kind::Failed(Box::new(ended)),
                });
            }
        }
    }
    let request_run = request
        .transition(transitions::APPROVE)
        .by_system()
        .meta(meta)
        .run_in(tx)
        .await
        .map_err(end::refused)?;
    ApprovalRequest::where_eq("id", request.id)
        .update(&mut *tx, &[("decided_at", &Some(renox::db::now()))])
        .await?;
    let request = ApprovalRequest::find(&mut *tx, request.id)
        .await?
        .ok_or(Error::NotFound)?;
    Ok(Approval {
        request,
        approvers,
        kind: Kind::Approved {
            request_run,
            subject_run,
        },
    })
}

/// 403 unless `user` may decide the current step of `request`: a holder of
/// one of its roles, or of its permission, in the subject's scope; with SoD
/// on, not the requester and not whoever decided another step.
pub async fn ensure_approver(db: &Db, request: &ApprovalRequest, user: &User) -> Result {
    let approvers = notify::step_approvers(db, request).await?;
    if approvers.iter().any(|approver| approver.id == user.id) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// `approver` approves the current step of request `id`, with an optional
/// `comment` (blank for none), commits, and announces it. On the last step
/// the action runs (`<transition>_approved`), or, when its guards no longer
/// pass, the request fails and the subject goes back. Returns
/// `ApprovalDecided` for an end, `None` when the request waits for its next
/// step. 403 when `approver` may not decide the step ([`ensure_approver`]);
/// 422 when the request was already decided.
pub async fn approve(
    state: &AppState,
    id: i64,
    approver: &User,
    comment: &str,
) -> Result<Option<ApprovalDecided>> {
    let request = end::pending(&state.db, id).await?;
    ensure_approver(&state.db, &request, approver).await?;
    let mut tx = state.db.begin().await?;
    let approval = approve_in(&mut tx, id, approver.id, request.current_step, comment).await?;
    tx.commit().await?;
    approval.announce(state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guard_or_a_moved_subject_fails_the_request() {
        assert_eq!(
            refusal_message(&TransitionError::Guard("over the balance".into())).as_deref(),
            Some("over the balance")
        );
        assert_eq!(
            refusal_message(&TransitionError::NotFrom("cancelled".into())).as_deref(),
            Some("this can't be done from the state `cancelled`")
        );
        assert_eq!(refusal_message(&TransitionError::Conflict), None);
        assert_eq!(refusal_message(&TransitionError::Unknown("x".into())), None);
    }

    #[test]
    fn a_refusal_comes_back_out_of_the_savepoint_with_its_message() {
        let error = into_savepoint_error(TransitionError::Guard("over the balance".into()));
        assert_eq!(refusal(error).ok().as_deref(), Some("over the balance"));
    }

    #[test]
    fn other_failures_stay_errors() {
        let conflict = into_savepoint_error(TransitionError::Conflict);
        assert!(matches!(
            refusal(conflict),
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
        let internal = Error::Internal(renox::anyhow::anyhow!("disk full"));
        assert!(matches!(refusal(internal), Err(Error::Internal(_))));
        assert!(matches!(refusal(Error::NotFound), Err(Error::NotFound)));
    }

    #[test]
    fn the_metadata_names_the_request_the_step_and_who_approved() {
        let request = ApprovalRequest {
            id: 9,
            requester_id: 1,
            current_step: 2,
            ..Default::default()
        };
        let meta = meta(&request, 4, &[3, 4]);
        assert_eq!(meta["approval_request"], 9);
        assert_eq!(meta["decision"], "approved");
        assert_eq!(meta["step"], 2);
        assert_eq!(meta["actor_id"], 4);
        assert_eq!(meta["requester_id"], 1);
        assert_eq!(meta["approvers"], json!([3, 4]));
    }

    #[test]
    fn the_fail_reason_carries_the_guard_message() {
        let end = End::fail(Some(4), "The amount exceeds the remaining balance.");
        assert_eq!(
            end.reason(),
            "approval failed: The amount exceeds the remaining balance."
        );
        assert_eq!(end.actor_id(), Some(4));
    }
}
