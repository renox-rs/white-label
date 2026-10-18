//! Submit (wl-kernel §4.1 `submit`, §6 `Approvals::require`): an action that
//! a policy blocks becomes a `pending` approval request.
//!
//! [`require_in`] is the one core, used two ways:
//!
//! - **without a waiting state**, the kernel's interceptor
//!   ([`super::intercept`]) calls it for a user's transition and defers the
//!   run: the status stays where it is;
//! - **with a waiting state**, the module's own transition into it (e.g.
//!   `requested → pending_approval`) calls it from its effect.
//!
//! Either way the request is created in the caller's transaction, with the
//! payload snapshot, a copy of the policy's steps and SoD flag, the expiry
//! time and the subject's state before; the module's reserve handler runs
//! in the same transaction. Once it commits, [`announce`] (or
//! [`announce_request`]) emits `ApprovalRequested` and tells the step-1
//! approvers.

use renox::chrono::Duration;
use renox::db::{Executor, Json, Transaction};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine};

use super::events::ApprovalRequested;
use super::notify;
use super::policy::ApprovalPolicy;
use super::request::{ApprovalRequest, ApprovalStatus};
use crate::machines;
use crate::subject::Subject;
use crate::texts;

/// The key of the request's id in the data of a deferred run
/// ([`renox::state_machine::Pending::data`]).
pub const REQUEST_KEY: &str = "approval_request";

/// An action asking to run: what [`require_in`] checks against the
/// policies.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Require {
    /// The policy's action key, e.g. `payments.refund`.
    pub action: String,
    /// The record the action is about.
    pub subject: Subject,
    /// The subject's transition the request is for; on approval the kernel
    /// runs `<transition>_approved`.
    pub transition: String,
    /// The transition's input (amounts, reason): the payload snapshot.
    pub payload: Value,
    /// The user who asks.
    pub requester_id: i64,
    /// The subject's state now (the `from` state of the run).
    pub state_before: String,
}

impl Require {
    /// `requester_id` asks to run `transition` (action `action`) on
    /// `subject`, now in `state_before`, with no payload.
    pub fn new(
        action: impl Into<String>,
        subject: Subject,
        transition: impl Into<String>,
        requester_id: i64,
        state_before: impl Into<String>,
    ) -> Self {
        Require {
            action: action.into(),
            subject,
            transition: transition.into(),
            payload: Value::Null,
            requester_id,
            state_before: state_before.into(),
        }
    }

    /// The transition's input, stored as the payload snapshot.
    pub fn payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }
}

/// What [`require_in`] decided.
#[derive(Debug, Clone)]
pub enum Required {
    /// No policy matches: the action runs now.
    Allowed,
    /// A request was created; the action waits for it.
    Pending(Box<ApprovalRequest>),
}

impl Required {
    /// The request, when one was created.
    pub fn request(&self) -> Option<&ApprovalRequest> {
        match self {
            Required::Allowed => None,
            Required::Pending(request) => Some(request),
        }
    }
}

/// Checks the action against the policies, inside `tx`: when one matches,
/// creates the `pending` request (steps and SoD copied from the policy,
/// `expires_at`, `subject_state_before`) and runs the module's reserve
/// handler, without committing. Announce it with [`announce_request`] after
/// the commit.
///
/// Refused with 422 when a request for the same subject and action is
/// already pending. Fails when the subject's registered machine has no
/// `<transition>_approved` transition to run on approval.
pub async fn require_in(tx: &mut Transaction, require: &Require) -> Result<Required> {
    let Some(policy) =
        ApprovalPolicy::for_action(&mut *tx, &require.action, &require.payload).await?
    else {
        return Ok(Required::Allowed);
    };
    if pending_for_action(&mut *tx, &require.subject, &require.action)
        .await?
        .is_some()
    {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::en("wl_kernel.approvals.already_pending"),
        ));
    }
    let machine = machines::get(&require.subject.subject_type);
    if let Some(machine) = &machine {
        let approved = machines::approved(&require.transition);
        if machine.transition(&approved).is_none() {
            return Err(renox::anyhow::anyhow!(
                "{}: an approval of `{}` needs a `.system()` transition `{approved}`",
                machine.subject_type(),
                require.transition
            )
            .into());
        }
    }
    let now = renox::db::now();
    let request = ApprovalRequest::create(&mut *tx, new_request(&policy, require, now)).await?;
    if let Some(machine) = &machine {
        machine.reserve_in(tx, &request.hold()).await?;
    }
    Ok(Required::Pending(Box::new(request)))
}

/// The row a submit inserts.
fn new_request(policy: &ApprovalPolicy, require: &Require, now: DateTime) -> ApprovalRequest {
    ApprovalRequest {
        policy_id: policy.id,
        subject_type: require.subject.subject_type.clone(),
        subject_id: require.subject.subject_id,
        action: require.action.clone(),
        transition: require.transition.clone(),
        payload: Json(require.payload.clone()),
        requester_id: require.requester_id,
        current_step: 1,
        steps: Json(policy.steps.0.clone()),
        sod: policy.sod,
        status: ApprovalStatus::Pending,
        expires_at: policy
            .expiry_hours
            .map(|hours| now + Duration::hours(hours)),
        subject_state_before: require.state_before.clone(),
        ..Default::default()
    }
}

/// The pending request about `subject` for `action`, if there is one.
pub async fn pending_for_action<'c>(
    db: impl Executor<'c>,
    subject: &Subject,
    action: &str,
) -> Result<Option<ApprovalRequest>> {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
        .where_(ApprovalRequest::ACTION.eq(action))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .first(db)
        .await
}

/// Announces the request a run created, after its transaction committed:
/// a run the interceptor deferred, or a run into the subject's waiting
/// state (whose effect called [`require_in`]). Any other run announces
/// nothing.
pub async fn announce<M>(state: &AppState, outcome: &Outcome<M>) -> Result
where
    M: StateMachine<Key = i64>,
{
    let request = match outcome {
        Outcome::Pending(pending) => match pending.data.get(REQUEST_KEY).and_then(Value::as_i64) {
            Some(id) => ApprovalRequest::find(&state.db, id).await?,
            None => None,
        },
        Outcome::Done(done) => {
            let Some(machine) = machines::by_table(M::TABLE) else {
                return Ok(());
            };
            if machine.waiting_state() != Some(done.to.as_str()) {
                return Ok(());
            }
            let subject = Subject::new(machine.subject_type(), done.key);
            ApprovalRequest::pending_about(&state.db, &subject)
                .await?
                .pop()
        }
    };
    match request {
        Some(request) => announce_request(state, &request).await,
        None => Ok(()),
    }
}

/// Emits `ApprovalRequested` for `request` and tells its step-1 approvers;
/// call it once the transaction that created the request has committed.
pub async fn announce_request(state: &AppState, request: &ApprovalRequest) -> Result {
    let approvers = notify::step_approvers(&state.db, request).await?;
    let step_approvers = approvers.iter().map(|user| user.id).collect();
    state
        .emit(ApprovalRequested {
            request: request.id,
            policy: request.policy_id,
            subject: request.subject(),
            action: request.action.clone(),
            requester: request.requester_id,
            step_approvers,
        })
        .await?;
    notify::requested(state, request, &approvers).await
}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;

    use super::*;
    use crate::approvals::policy::Step;

    fn policy(expiry_hours: Option<i64>) -> ApprovalPolicy {
        ApprovalPolicy {
            id: 4,
            action: "payments.refund".into(),
            steps: Json(vec![
                Step::one_of(["property_manager"]),
                Step::one_of(["finance", "owner"]),
            ]),
            sod: true,
            expiry_hours,
            active: true,
            ..Default::default()
        }
    }

    fn require() -> Require {
        Require::new(
            "payments.refund",
            Subject::new("payments.refund", 9),
            "refund",
            2,
            "requested",
        )
        .payload(json!({ "amount": 50_000 }))
    }

    #[test]
    fn a_request_copies_the_policy_and_the_run() {
        let now = renox::db::now();
        let row = new_request(&policy(Some(72)), &require(), now);
        assert_eq!(row.policy_id, 4);
        assert_eq!(row.subject(), Subject::new("payments.refund", 9));
        assert_eq!(row.transition, "refund");
        assert_eq!(row.payload.0["amount"], 50_000);
        assert_eq!(row.requester_id, 2);
        assert_eq!(row.current_step, 1);
        assert_eq!(row.steps.len(), 2);
        assert!(row.steps[1].allows("owner"));
        assert!(row.sod);
        assert!(row.is_pending());
        assert_eq!(row.subject_state_before, "requested");
        assert_eq!(row.expires_at, Some(now + Duration::hours(72)));
        assert_eq!(row.released_at, None);
    }

    #[test]
    fn a_policy_without_expiry_gives_a_request_that_never_expires() {
        let row = new_request(&policy(None), &require(), renox::db::now());
        assert_eq!(row.expires_at, None);
    }

    #[test]
    fn sod_is_copied_from_the_policy() {
        let mut off = policy(None);
        off.sod = false;
        assert!(!new_request(&off, &require(), renox::db::now()).sod);
    }

    #[test]
    fn allowed_has_no_request() {
        assert!(Required::Allowed.request().is_none());
        let pending = Required::Pending(Box::new(new_request(
            &policy(None),
            &require(),
            renox::db::now(),
        )));
        assert_eq!(pending.request().map(|r| r.subject_id), Some(9));
    }
}
