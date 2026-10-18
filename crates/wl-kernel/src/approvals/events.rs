//! The approval events (wl-kernel §6), emitted after the commit.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// A request was created: an action waits for approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequested {
    /// The request's id.
    pub request: i64,
    /// The policy that matched.
    pub policy: i64,
    /// The record the request is about.
    pub subject: Subject,
    /// The action key, e.g. `payments.refund`.
    pub action: String,
    /// The user who asked.
    pub requester: i64,
    /// The users who may decide step 1.
    pub step_approvers: Vec<i64>,
}

impl Event for ApprovalRequested {}

/// How a request ended.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalEnd {
    /// All steps approved; the action ran.
    #[default]
    Approved,
    /// An approver said no.
    Rejected,
    /// An approver sent it back for changes.
    Returned,
    /// The requester cancelled it.
    Withdrawn,
    /// No decision before the expiry time.
    Expired,
    /// Approved, but the action could not run.
    Failed,
}

/// A request ended, whatever the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecided {
    /// The request's id.
    pub request: i64,
    /// How it ended.
    pub decision: ApprovalEnd,
    /// The users who decided (every step's approver, in order); empty for
    /// an end nobody decided (`withdraw`, `expire`).
    pub approvers: Vec<i64>,
    /// The record the request is about.
    pub subject: Subject,
    /// The state the subject went back to; `None` when it did not move (no
    /// waiting state, already moved, or approved).
    pub state_returned_to: Option<String>,
}

impl Event for ApprovalDecided {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decided_serializes_the_end_in_snake_case() {
        let decided = ApprovalDecided {
            request: 4,
            decision: ApprovalEnd::Rejected,
            approvers: vec![2],
            subject: Subject::new("payments.refund", 9),
            state_returned_to: Some("requested".into()),
        };
        let json = renox::serde_json::to_value(&decided).unwrap();
        assert_eq!(json["decision"], "rejected");
        assert_eq!(json["state_returned_to"], "requested");
        assert_eq!(json["subject"]["subject_type"], "payments.refund");
    }
}
