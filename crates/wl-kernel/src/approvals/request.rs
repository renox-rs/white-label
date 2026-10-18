//! Approval requests and decisions (wl-kernel §3, §4.1).
//!
//! ```text
//! [*] --> pending --approve_step--> pending
//!         pending --approve--> approved
//!         pending --reject | withdraw | expire | fail--> rejected | withdrawn | expired | failed
//!         pending --return--> returned            (#26)
//! ```
//!
//! Every transition of a request is `.system()`: the kernel's approval
//! functions check who acts (an approver of the current step in the
//! subject's scope, the requester for `withdraw`) and store that user in the
//! history row's metadata. A request is inserted only as `pending`.

use renox::db::{Executor, Json};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};

use super::events::ApprovalEnd;
use super::policy::Step;
use crate::machines::Hold;
use crate::subject::Subject;

/// The transition names of a request.
pub mod transitions {
    /// An approver of a step that is not the last approves it.
    pub const APPROVE_STEP: &str = "approve_step";
    /// An approver of the last step approves it.
    pub const APPROVE: &str = "approve";
    /// The action's guards no longer pass at final approval.
    pub const FAIL: &str = "fail";
    /// An approver of the current step says no.
    pub const REJECT: &str = "reject";
    /// The requester cancels it.
    pub const WITHDRAW: &str = "withdraw";
    /// No decision before the expiry time.
    pub const EXPIRE: &str = "expire";
}

/// The state of an approval request.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalStatus {
    /// Waiting for the approver(s) of the current step.
    #[default]
    Pending,
    /// All steps approved; the action has run.
    Approved,
    /// An approver rejected it: the answer is no.
    Rejected,
    /// An approver sent it back for changes.
    Returned,
    /// The requester cancelled it.
    Withdrawn,
    /// No decision before the expiry time.
    Expired,
    /// Approved, but the action could not run because the record changed.
    Failed,
}

impl ApprovalStatus {
    /// How the request ended; `None` while it is pending.
    pub fn end(self) -> Option<ApprovalEnd> {
        match self {
            ApprovalStatus::Pending => None,
            ApprovalStatus::Approved => Some(ApprovalEnd::Approved),
            ApprovalStatus::Rejected => Some(ApprovalEnd::Rejected),
            ApprovalStatus::Returned => Some(ApprovalEnd::Returned),
            ApprovalStatus::Withdrawn => Some(ApprovalEnd::Withdrawn),
            ApprovalStatus::Expired => Some(ApprovalEnd::Expired),
            ApprovalStatus::Failed => Some(ApprovalEnd::Failed),
        }
    }
}

/// One pending (or ended) decision about an action on a subject.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_approval_requests",
    state_machine,
    index(subject_type, subject_id, status)
)]
pub struct ApprovalRequest {
    /// The primary key.
    pub id: i64,
    /// The policy that matched at submit.
    pub policy_id: i64,
    /// The subject's type, e.g. `payments.refund`.
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: i64,
    /// The policy's action key, e.g. `payments.refund`.
    pub action: String,
    /// The subject's transition the request is for, e.g. `refund`; on
    /// approval the kernel runs `<transition>_approved`.
    pub transition: String,
    /// The transition's input at submit (amounts, reason): the payload
    /// snapshot. Never changed afterwards.
    pub payload: Json<Value>,
    /// The user who asked.
    pub requester_id: i64,
    /// The step waiting for a decision, from 1.
    pub current_step: i64,
    /// The policy's steps, copied at submit: later policy edits do not
    /// change a pending request.
    pub steps: Json<Vec<Step>>,
    /// Segregation of duties, copied from the policy at submit.
    pub sod: bool,
    /// The state; changed only by a transition.
    pub status: ApprovalStatus,
    /// When the request may expire (an instant, shown in Asia/Dubai);
    /// `None` for never.
    pub expires_at: Option<DateTime>,
    /// The subject's state at submit.
    pub subject_state_before: String,
    /// The state the subject was in at the end, when it had already left its
    /// waiting state ("subject already moved to `<state>`").
    pub subject_moved_to: Option<String>,
    /// The state the subject went back to at a non-yes end, if it moved.
    pub state_returned_to: Option<String>,
    /// When the module's reservation was released; set once.
    pub released_at: Option<DateTime>,
    /// The earlier returned or rejected request this one resubmits.
    pub resubmission_of: Option<i64>,
    /// When the request ended.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ApprovalRequest {
    /// The record the request is about.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// Whether the request still waits for a decision.
    pub fn is_pending(&self) -> bool {
        self.status == ApprovalStatus::Pending
    }

    /// The step waiting for a decision, if the request has it.
    pub fn step(&self) -> Option<&Step> {
        let index = usize::try_from(self.current_step.checked_sub(1)?).ok()?;
        self.steps.get(index)
    }

    /// Whether the current step is the last one.
    pub fn on_last_step(&self) -> bool {
        usize::try_from(self.current_step).is_ok_and(|step| step >= self.steps.len())
    }

    /// Whether the expiry time has passed.
    pub fn is_past_expiry(&self, now: DateTime) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }

    /// What the module's reserve and release handlers are told.
    pub fn hold(&self) -> Hold {
        Hold::new(
            self.id,
            self.subject(),
            self.action.clone(),
            self.payload.0.clone(),
        )
    }

    /// The pending requests about `subject`, oldest first (what the
    /// pending-approval badge shows).
    pub async fn pending_about<'c>(
        db: impl Executor<'c>,
        subject: &Subject,
    ) -> Result<Vec<ApprovalRequest>> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
            .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
            .order_by(ApprovalRequest::ID)
            .get(db)
            .await
    }

    /// The request's decisions, in the order they were taken.
    pub async fn decisions<'c>(&self, db: impl Executor<'c>) -> Result<Vec<ApprovalDecision>> {
        ApprovalDecision::query()
            .where_(ApprovalDecision::REQUEST_ID.eq(self.id))
            .order_by(ApprovalDecision::ID)
            .get(db)
            .await
    }
}

impl StateMachine for ApprovalRequest {
    type State = ApprovalStatus;
    const COLUMN: renox::db::Col<ApprovalRequest, ApprovalStatus> = ApprovalRequest::STATUS;

    fn machine() -> Machine<Self> {
        use ApprovalStatus::{Approved, Expired, Failed, Pending, Rejected, Returned, Withdrawn};
        Machine::new()
            .initial([Pending])
            .state(Pending)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Returned)
            .tone("warning")
            .terminal()
            .state(Withdrawn)
            .tone("neutral")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Failed)
            .tone("error")
            .terminal()
            .transition(
                Transition::new(transitions::APPROVE_STEP)
                    .label("Approve step")
                    .from([Pending])
                    .to(Pending)
                    .system()
                    .guard("not_last_step", |request: &ApprovalRequest, _ctx| {
                        Box::pin(async move {
                            if request.on_last_step() {
                                Err("This is the last step: approve the request.".into())
                            } else {
                                Ok(())
                            }
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::APPROVE)
                    .label("Approve")
                    .from([Pending])
                    .to(Approved)
                    .system()
                    .guard("last_step", |request: &ApprovalRequest, _ctx| {
                        Box::pin(async move {
                            if request.on_last_step() {
                                Ok(())
                            } else {
                                Err("Other steps still need approval.".into())
                            }
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::FAIL)
                    .label("Fail")
                    .from([Pending])
                    .to(Failed)
                    .system(),
            )
            .transition(
                Transition::new(transitions::REJECT)
                    .label("Reject")
                    .from([Pending])
                    .to(Rejected)
                    .system()
                    .reason_required(),
            )
            .transition(
                Transition::new(transitions::WITHDRAW)
                    .label("Withdraw")
                    .from([Pending])
                    .to(Withdrawn)
                    .system(),
            )
            .transition(
                Transition::new(transitions::EXPIRE)
                    .label("Expire")
                    .from([Pending])
                    .to(Expired)
                    .system()
                    .guard("past_expiry", |request: &ApprovalRequest, _ctx| {
                        Box::pin(async move {
                            if request.is_past_expiry(renox::db::now()) {
                                Ok(())
                            } else {
                                Err("The request has not reached its expiry time.".into())
                            }
                        })
                    }),
            )
    }
}

/// An approver's answer.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DecisionKind {
    /// Yes, for this step.
    #[default]
    Approve,
    /// No.
    Reject,
    /// Sent back for changes (#26).
    Return,
}

/// One approver's answer to one step of a request.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_approval_decisions", index(request_id))]
pub struct ApprovalDecision {
    /// The primary key.
    pub id: i64,
    /// The request.
    pub request_id: i64,
    /// The step decided, from 1.
    pub step: i64,
    /// The user who decided.
    pub approver_id: i64,
    /// The answer.
    pub decision: DecisionKind,
    /// The approver's comment (required to reject or return).
    pub comment: Option<String>,
    /// When the decision was taken.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use renox::state_machine::info_of;

    use super::*;

    fn request(steps: usize, current_step: i64) -> ApprovalRequest {
        ApprovalRequest {
            id: 7,
            subject_type: "payments.refund".into(),
            subject_id: 3,
            action: "payments.refund".into(),
            transition: "refund".into(),
            current_step,
            steps: Json(vec![Step::one_of(["finance"]); steps]),
            ..Default::default()
        }
    }

    #[test]
    fn every_transition_is_system_and_only_reject_needs_a_reason() {
        let info = info_of::<ApprovalRequest>();
        assert_eq!(info.states.len(), 7);
        assert_eq!(info.transitions.len(), 6);
        for t in &info.transitions {
            assert!(t.system, "`{}` must be system", t.name);
            assert_eq!(t.needs_reason, t.name == transitions::REJECT, "{}", t.name);
        }
        assert!(info.transitions.iter().all(|t| t.name != "return"));
    }

    #[test]
    fn steps_are_counted_from_one() {
        let two = request(2, 1);
        assert_eq!(two.step(), Some(&Step::one_of(["finance"])));
        assert!(!two.on_last_step());
        assert!(request(2, 2).on_last_step());
        assert!(request(1, 1).on_last_step());
        assert_eq!(request(1, 0).step(), None);
    }

    #[test]
    fn only_a_request_with_an_expiry_time_expires() {
        let now = renox::db::now();
        let mut request = request(1, 1);
        assert!(!request.is_past_expiry(now));
        request.expires_at = Some(now);
        assert!(request.is_past_expiry(now));
        request.expires_at = Some(now + renox::chrono::Duration::hours(1));
        assert!(!request.is_past_expiry(now));
    }

    #[test]
    fn the_hold_carries_the_request_and_its_payload() {
        let mut request = request(1, 1);
        request.payload = Json(renox::serde_json::json!({ "amount": 50_000 }));
        let hold = request.hold();
        assert_eq!(hold.request_id, 7);
        assert_eq!(hold.subject, Subject::new("payments.refund", 3));
        assert_eq!(hold.payload["amount"], 50_000);
    }

    #[test]
    fn ended_states_map_to_their_end() {
        assert_eq!(ApprovalStatus::Pending.end(), None);
        assert_eq!(ApprovalStatus::Rejected.end(), Some(ApprovalEnd::Rejected));
        assert_eq!(ApprovalStatus::Failed.end(), Some(ApprovalEnd::Failed));
    }
}
