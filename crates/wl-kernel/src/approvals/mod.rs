//! Approvals (wl-kernel §3, §4.1, `PLT.APR.01`): the policies that say who
//! must approve an action, the requests waiting for a decision, the
//! approvers' decisions, and the events `ApprovalRequested` and
//! `ApprovalDecided`.
//!
//! Tables: `wl_approval_policies`, `wl_approval_requests`,
//! `wl_approval_decisions`. A request's status moves only through its
//! machine ([`ApprovalRequest`]), whose transitions are all `system`.

pub mod decide;
pub mod end;
pub mod events;
pub mod intercept;
pub mod notify;
pub mod policy;
pub mod request;
pub mod submit;
pub mod web;

use renox::prelude::*;

pub use decide::{Approval, approve, approve_in, ensure_approver};
pub use end::{End, Ended, SubjectEnd, end_in, expire, reject, withdraw};
pub use events::{ApprovalDecided, ApprovalEnd, ApprovalRequested};
pub use intercept::ApprovalInterceptor;
pub use policy::{ApprovalPolicy, Condition, ConditionKind, PolicyDefault, Step};
pub use request::{ApprovalDecision, ApprovalRequest, ApprovalStatus, DecisionKind};
pub use submit::{Require, Required, announce, announce_request, require_in};
pub use web::{ApprovalCard, card, cards_for, pending_for, timeline_for};

/// Registers the approval models and the request's machine, and installs
/// the interceptor that turns blocked user transitions into requests.
pub(crate) fn register(app: &mut Registry) {
    app.model::<ApprovalPolicy>()
        .model::<ApprovalRequest>()
        .model::<ApprovalDecision>();
    renox::state_machine::register::<ApprovalRequest>();
    renox::state_machine::set_interceptor(ApprovalInterceptor);
}

/// Stores the default policies the modules registered and the database
/// lacks; see [`policy::install`].
pub async fn install(db: &Db) -> Result<usize> {
    policy::install(db).await
}
