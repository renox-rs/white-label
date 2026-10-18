//! The ends that are not a yes (wl-kernel §4.1 `reject`, `withdraw`,
//! `expire`, `fail`; "What happens to the record when the answer is not
//! yes", `PLT.WFL.01`).
//!
//! Every such end goes through one path, [`end_in`], inside one
//! transaction:
//!
//! 1. the request row is loaded locked; one that is no longer `pending` is
//!    refused ("this request was already decided"), and so is a second end
//!    racing the first (the engine's optimistic update gives `Conflict`);
//! 2. the request's own `system` transition runs, with the history reason
//!    (the comment for `reject`, a fixed text otherwise);
//! 3. the subject: when it is in its waiting state, the kernel runs its
//!    `back_to_<state before>` with the same reason; when it is somewhere
//!    else than its waiting state and its state before, the request records
//!    "subject already moved to `<state>`" and nothing moves; a subject
//!    without a waiting state never moves;
//! 4. the module's release handler runs (once: `released_at` is set), in the
//!    same transaction.
//!
//! If any step fails the whole end rolls back. Once the transaction has
//! committed, [`Ended::announce`] emits the engine's `Transitioned` events,
//! `ApprovalDecided`, and the notices. [`reject`], [`withdraw`] and
//! [`expire`] do all of it; `fail` (#25 part 5) and `return` (#26) call
//! [`end_in`] from their own transaction.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, TransitionError};

use super::events::{ApprovalDecided, ApprovalEnd};
use super::notify;
use super::request::{ApprovalDecision, ApprovalRequest, DecisionKind, transitions};
use crate::machines::{self, Ran, Run};
use crate::texts;

/// An end that is not a yes: how the request ends, who ended it, and the
/// history reason. Built with [`End::reject`], [`End::withdraw`],
/// [`End::expire`] or [`End::fail`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct End {
    kind: ApprovalEnd,
    actor_id: Option<i64>,
    reason: String,
    comment: Option<String>,
    at_step: Option<i64>,
}

impl End {
    /// An approver of step `step` says no, with `comment` (required; it is
    /// the history reason).
    pub fn reject(approver_id: i64, step: i64, comment: &str) -> Self {
        let comment = comment.trim().to_owned();
        End {
            kind: ApprovalEnd::Rejected,
            actor_id: Some(approver_id),
            reason: comment.clone(),
            comment: Some(comment),
            at_step: Some(step),
        }
    }

    /// The requester cancels the request.
    pub fn withdraw(requester_id: i64) -> Self {
        End {
            kind: ApprovalEnd::Withdrawn,
            actor_id: Some(requester_id),
            reason: texts::en("wl_kernel.approvals.reason_withdrawn"),
            comment: None,
            at_step: None,
        }
    }

    /// No decision before the expiry time.
    pub fn expire() -> Self {
        End {
            kind: ApprovalEnd::Expired,
            actor_id: None,
            reason: texts::en("wl_kernel.approvals.reason_expired"),
            comment: None,
            at_step: None,
        }
    }

    /// The final approval came, but the action's guards no longer pass:
    /// `guard_message` says why. `approver_id` is who approved last.
    pub fn fail(approver_id: Option<i64>, guard_message: &str) -> Self {
        End {
            kind: ApprovalEnd::Failed,
            actor_id: approver_id,
            reason: texts::en_with(
                "wl_kernel.approvals.reason_failed",
                &[("message", &guard_message)],
            ),
            comment: None,
            at_step: None,
        }
    }

    /// How the request ends.
    pub fn kind(&self) -> ApprovalEnd {
        self.kind
    }

    /// Who ended it; `None` for the platform (`expire`).
    pub fn actor_id(&self) -> Option<i64> {
        self.actor_id
    }

    /// The history reason, on the request's and the subject's rows.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The request's transition for this end.
    fn transition(&self) -> &'static str {
        match self.kind {
            ApprovalEnd::Rejected => transitions::REJECT,
            ApprovalEnd::Withdrawn => transitions::WITHDRAW,
            ApprovalEnd::Expired => transitions::EXPIRE,
            ApprovalEnd::Failed => transitions::FAIL,
            ApprovalEnd::Approved | ApprovalEnd::Returned => {
                unreachable!("an `End` is only built for reject, withdraw, expire or fail")
            }
        }
    }

    /// The history rows' metadata: the request, the end and who acted.
    fn meta(&self, request_id: i64) -> Value {
        json!({
            "approval_request": request_id,
            "decision": self.kind,
            "actor_id": self.actor_id,
        })
    }
}

/// What the end did to the subject.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubjectEnd {
    /// The state it went back to, when the kernel moved it.
    pub returned_to: Option<String>,
    /// The state it was found in, when it had already moved.
    pub moved_to: Option<String>,
}

/// What to do with the subject, from its state now: `waiting` is its
/// machine's waiting state, `before` the state it had at submit.
fn subject_end(current: Option<&str>, waiting: Option<&str>, before: &str) -> SubjectAction {
    match current {
        None => SubjectAction::Leave,
        Some(current) if waiting == Some(current) => SubjectAction::Back,
        Some(current) if current == before => SubjectAction::Leave,
        Some(current) => SubjectAction::Moved(current.to_owned()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SubjectAction {
    /// Run `back_to_<state before>`.
    Back,
    /// Nothing to do: it is where the action found it (or gone).
    Leave,
    /// It left its waiting state by another path: record where it is.
    Moved(String),
}

/// A request ended in a transaction that has not committed yet. Call
/// [`Ended::announce`] once it has.
#[must_use = "announce it after the transaction commits"]
pub struct Ended {
    /// The request, as it is after the end.
    pub request: ApprovalRequest,
    /// The end.
    pub end: End,
    /// What happened to the subject.
    pub subject: SubjectEnd,
    /// The users who decided a step of the request, in order.
    pub approvers: Vec<i64>,
    request_run: Outcome<ApprovalRequest>,
    subject_run: Option<Ran>,
}

impl std::fmt::Debug for Ended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ended")
            .field("request", &self.request.id)
            .field("end", &self.end)
            .field("subject", &self.subject)
            .field("approvers", &self.approvers)
            .finish()
    }
}

impl Ended {
    /// The event this end emits.
    pub fn decided(&self) -> ApprovalDecided {
        ApprovalDecided {
            request: self.request.id,
            decision: self.end.kind,
            approvers: self.approvers.clone(),
            subject: self.request.subject(),
            state_returned_to: self.subject.returned_to.clone(),
        }
    }

    /// After the commit: emits `Transitioned` for the request (and the
    /// subject, when it moved back), then `ApprovalDecided`, then tells the
    /// people the end concerns. Returns the event.
    pub async fn announce(self, state: &AppState) -> Result<ApprovalDecided> {
        let decided = self.decided();
        self.request_run.emit(state).await?;
        if let Some(run) = self.subject_run {
            run.emit(state).await?;
        }
        state.emit(decided.clone()).await?;
        notify::decided(state, &self.request, &self.end, &self.subject).await?;
        Ok(decided)
    }
}

/// The 422 for a request that is no longer pending.
fn already_decided() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        texts::en("wl_kernel.approvals.already_decided"),
    )
}

/// The engine's refusal of the request's own transition, as an error.
fn refused(error: TransitionError) -> Error {
    match error {
        TransitionError::NotFrom(_) | TransitionError::Conflict => already_decided(),
        TransitionError::ReasonRequired => comment_required(),
        other => other.into_error(),
    }
}

fn comment_required() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        texts::en("wl_kernel.approvals.comment_required"),
    )
}

/// Request `id`, loaded in `tx` and locked (PostgreSQL) until it ends.
async fn lock(tx: &mut Transaction, id: i64) -> Result<ApprovalRequest> {
    ApprovalRequest::query()
        .where_eq("id", id)
        .lock_for_update()
        .first(&mut *tx)
        .await?
        .ok_or(Error::NotFound)
}

/// Ends request `id` with `end`, inside `tx`, without committing (see the
/// module's steps). Refused with 422 when the request is no longer pending
/// (or its step moved on since the caller checked), when a reject has no
/// comment, or when `expire` comes before the expiry time. Any failure,
/// the module's release handler included, leaves `tx` to be rolled back.
pub async fn end_in(tx: &mut Transaction, id: i64, end: End) -> Result<Ended> {
    if end.kind == ApprovalEnd::Rejected && end.reason.is_empty() {
        return Err(comment_required());
    }
    let request = lock(tx, id).await?;
    if !request.is_pending() || end.at_step.is_some_and(|step| step != request.current_step) {
        return Err(already_decided());
    }
    let meta = end.meta(request.id);
    let mut run = request
        .transition(end.transition())
        .by_system()
        .meta(meta.clone());
    if !end.reason.is_empty() {
        run = run.reason(end.reason.clone());
    }
    let request_run = run.run_in(tx).await.map_err(refused)?;

    if end.kind == ApprovalEnd::Rejected
        && let Some(approver_id) = end.actor_id
    {
        ApprovalDecision::create(
            &mut *tx,
            ApprovalDecision {
                request_id: request.id,
                step: request.current_step,
                approver_id,
                decision: DecisionKind::Reject,
                comment: end.comment.clone(),
                ..Default::default()
            },
        )
        .await?;
    }

    let mut subject = SubjectEnd::default();
    let mut subject_run = None;
    let machine = machines::get(&request.subject_type);
    if let Some(machine) = &machine {
        let current = machine.current_state(&mut *tx, request.subject_id).await?;
        match subject_end(
            current.as_deref(),
            machine.waiting_state(),
            &request.subject_state_before,
        ) {
            SubjectAction::Back => {
                let run = Run::new().reason(end.reason.clone()).meta(meta);
                let ran = machine
                    .run_back_to(tx, request.subject_id, &request.subject_state_before, run)
                    .await
                    .map_err(TransitionError::into_error)?;
                subject.returned_to = Some(request.subject_state_before.clone());
                subject_run = Some(ran);
            }
            SubjectAction::Moved(state) => subject.moved_to = Some(state),
            SubjectAction::Leave => {}
        }
    }

    let now = renox::db::now();
    let released_at = match request.released_at {
        Some(at) => at,
        None => {
            if let Some(machine) = &machine {
                machine.release_in(tx, &request.hold()).await?;
            }
            now
        }
    };
    ApprovalRequest::where_eq("id", request.id)
        .update(
            &mut *tx,
            &[
                ("subject_moved_to", &subject.moved_to),
                ("state_returned_to", &subject.returned_to),
                ("released_at", &Some(released_at)),
                ("decided_at", &Some(now)),
            ],
        )
        .await?;
    let request = ApprovalRequest::find(&mut *tx, request.id)
        .await?
        .ok_or(Error::NotFound)?;
    let approvers = request
        .decisions(&mut *tx)
        .await?
        .into_iter()
        .map(|decision| decision.approver_id)
        .collect();
    Ok(Ended {
        request,
        end,
        subject,
        approvers,
        request_run,
        subject_run,
    })
}

/// Runs `end` on request `id` in its own transaction, commits, and
/// announces it.
async fn end_now(state: &AppState, id: i64, end: End) -> Result<ApprovalDecided> {
    let mut tx = state.db.begin().await?;
    let ended = end_in(&mut tx, id, end).await?;
    tx.commit().await?;
    ended.announce(state).await
}

/// The pending request `id`; 422 when it already ended.
async fn pending(db: &Db, id: i64) -> Result<ApprovalRequest> {
    let request = ApprovalRequest::find(db, id)
        .await?
        .ok_or(Error::NotFound)?;
    if !request.is_pending() {
        return Err(already_decided());
    }
    Ok(request)
}

/// `approver` rejects request `id` with `comment` (required, not blank):
/// the subject goes back to its state before, the module's reservation is
/// released, the requester is told. `approver` must be one of the current
/// step's approvers (a holder of one of its roles in the subject's scope,
/// SoD applied): 403 otherwise.
pub async fn reject(
    state: &AppState,
    id: i64,
    approver: &User,
    comment: &str,
) -> Result<ApprovalDecided> {
    if comment.trim().is_empty() {
        return Err(comment_required());
    }
    let request = pending(&state.db, id).await?;
    let approvers = notify::step_approvers(&state.db, &request).await?;
    if !approvers.iter().any(|user| user.id == approver.id) {
        return Err(Error::Forbidden);
    }
    end_now(
        state,
        id,
        End::reject(approver.id, request.current_step, comment),
    )
    .await
}

/// The requester withdraws request `id`: the subject goes back to its state
/// before, the reservation is released, the current step's approvers are
/// told. Anyone else gets 403.
pub async fn withdraw(state: &AppState, id: i64, requester: &User) -> Result<ApprovalDecided> {
    let request = pending(&state.db, id).await?;
    if request.requester_id != requester.id {
        return Err(Error::Forbidden);
    }
    end_now(state, id, End::withdraw(requester.id)).await
}

/// Request `id` expires (run by the platform once `expires_at` has passed;
/// refused with 422 before): the subject goes back to its state before, the
/// reservation is released, the requester is told.
pub async fn expire(state: &AppState, id: i64) -> Result<ApprovalDecided> {
    end_now(state, id, End::expire()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reject_carries_the_trimmed_comment_as_its_reason() {
        let end = End::reject(4, 2, "  duplicate ");
        assert_eq!(end.kind(), ApprovalEnd::Rejected);
        assert_eq!(end.reason(), "duplicate");
        assert_eq!(end.comment.as_deref(), Some("duplicate"));
        assert_eq!(end.at_step, Some(2));
        assert_eq!(end.transition(), transitions::REJECT);
    }

    #[test]
    fn ends_without_a_comment_have_fixed_reasons() {
        assert_eq!(End::withdraw(3).reason(), "approval withdrawn by requester");
        assert_eq!(End::expire().reason(), "approval expired");
        assert_eq!(
            End::fail(Some(5), "The amount exceeds the remaining balance.").reason(),
            "approval failed: The amount exceeds the remaining balance."
        );
        assert_eq!(End::withdraw(3).transition(), transitions::WITHDRAW);
        assert_eq!(End::expire().transition(), transitions::EXPIRE);
        assert_eq!(End::fail(None, "x").transition(), transitions::FAIL);
    }

    #[test]
    fn the_history_metadata_names_the_request_the_end_and_the_actor() {
        let meta = End::reject(4, 1, "duplicate").meta(9);
        assert_eq!(meta["approval_request"], 9);
        assert_eq!(meta["decision"], "rejected");
        assert_eq!(meta["actor_id"], 4);
        assert!(End::expire().meta(9)["actor_id"].is_null());
    }

    #[test]
    fn a_subject_in_its_waiting_state_goes_back() {
        assert_eq!(
            subject_end(
                Some("pending_approval"),
                Some("pending_approval"),
                "requested"
            ),
            SubjectAction::Back
        );
    }

    #[test]
    fn a_subject_that_moved_elsewhere_is_left_where_it_is() {
        assert_eq!(
            subject_end(Some("cancelled"), Some("pending_approval"), "requested"),
            SubjectAction::Moved("cancelled".into())
        );
        assert_eq!(
            subject_end(Some("cancelled"), None, "requested"),
            SubjectAction::Moved("cancelled".into())
        );
    }

    #[test]
    fn a_subject_without_a_waiting_state_never_moves() {
        assert_eq!(
            subject_end(Some("requested"), None, "requested"),
            SubjectAction::Leave
        );
        assert_eq!(
            subject_end(Some("active"), Some("pending_approval"), "active"),
            SubjectAction::Leave
        );
        assert_eq!(
            subject_end(None, Some("pending_approval"), "requested"),
            SubjectAction::Leave
        );
    }

    #[test]
    fn the_engine_refusals_of_a_second_end_read_already_decided() {
        let message = |error: Error| match error {
            Error::Status(code, message) => (code, message),
            other => panic!("unexpected {other:?}"),
        };
        let decided = (
            StatusCode::UNPROCESSABLE_ENTITY,
            "This request was already decided.".to_owned(),
        );
        assert_eq!(message(refused(TransitionError::Conflict)), decided);
        assert_eq!(
            message(refused(TransitionError::NotFrom("rejected".into()))),
            decided
        );
        assert_eq!(
            message(refused(TransitionError::ReasonRequired)).1,
            "A comment is required to reject."
        );
    }
}
