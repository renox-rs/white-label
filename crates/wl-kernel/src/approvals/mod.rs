//! Approvals (wl-kernel §3, §4.1, `PLT.APR.01`): the policies that say who
//! must approve an action, the requests waiting for a decision, the
//! approvers' decisions, and the events `ApprovalRequested` and
//! `ApprovalDecided`.
//!
//! Tables: `wl_approval_policies`, `wl_approval_requests`,
//! `wl_approval_decisions`. A request's status moves only through its
//! machine ([`ApprovalRequest`]), whose transitions are all `system`.

pub mod events;
pub mod policy;
pub mod request;

use renox::prelude::*;

pub use events::{ApprovalDecided, ApprovalEnd, ApprovalRequested};
pub use policy::{ApprovalPolicy, Condition, ConditionKind, PolicyDefault, Step};
pub use request::{ApprovalDecision, ApprovalRequest, ApprovalStatus, DecisionKind};

/// Registers the approval models and the request's machine.
pub(crate) fn register(app: &mut Registry) {
    app.model::<ApprovalPolicy>()
        .model::<ApprovalRequest>()
        .model::<ApprovalDecision>();
    renox::state_machine::register::<ApprovalRequest>();
}

/// Stores the default policies the modules registered and the database
/// lacks; see [`policy::install`].
pub async fn install(db: &Db) -> Result<usize> {
    policy::install(db).await
}
