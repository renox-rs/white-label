//! Who decides a request, and the notices the kernel sends about it
//! (wl-kernel §4.1 effects).
//!
//! wl-notify is not built yet, so the kernel sends Renox notifications to
//! users directly, on the database channel (the in-app bell), after the
//! commit. Approvers are the holders of one of the current step's roles in
//! the subject's scope (from its registered machine), or globally when the
//! machine gives no scope.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::Value;

use super::request::ApprovalRequest;
use crate::machines;
use crate::subject::Subject;
use crate::texts;

/// The kind of the notice to approvers that a request waits for them.
pub const REQUESTED_KIND: &str = "approval-requested";

/// The scope approvers of `request` are looked for in: the subject's, from
/// its registered machine; global when it has none.
pub async fn scope_of(db: &Db, request: &ApprovalRequest) -> Result<Scope> {
    let Some(machine) = machines::get(&request.subject_type).filter(|m| m.has_scope()) else {
        return Ok(Scope::global());
    };
    let mut tx = db.begin().await?;
    let scope = machine.scope_in(&mut tx, request.subject_id).await?;
    tx.commit().await?;
    Ok(scope)
}

/// The users who may decide the request's current step: holders of one of
/// its roles in the subject's scope, ordered by id. With SoD on, the
/// requester and whoever decided another step of the request are left out.
pub async fn step_approvers(db: &Db, request: &ApprovalRequest) -> Result<Vec<User>> {
    let Some(step) = request.step() else {
        return Ok(Vec::new());
    };
    let scope = scope_of(db, request).await?;
    let mut excluded = BTreeSet::new();
    if request.sod {
        excluded.insert(request.requester_id);
        for decision in request.decisions(db).await? {
            if decision.step != request.current_step {
                excluded.insert(decision.approver_id);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut approvers = Vec::new();
    for role in &step.roles {
        for user in users_with_role_in(db, role, &scope).await? {
            if !excluded.contains(&user.id) && seen.insert(user.id) {
                approvers.push(user);
            }
        }
    }
    approvers.sort_by_key(|user| user.id);
    Ok(approvers)
}

/// Tells `approvers` that `request` waits for their decision.
pub async fn requested(state: &AppState, request: &ApprovalRequest, approvers: &[User]) -> Result {
    let notice = Requested::of(request);
    for approver in approvers {
        state.notify(approver, &notice).await?;
    }
    Ok(())
}

/// The notice to approvers: a request waits for their decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    /// The request's id.
    pub request: i64,
    /// The action key.
    pub action: String,
    /// The record the request is about.
    pub subject: Subject,
}

impl Requested {
    /// The notice about `request`.
    pub fn of(request: &ApprovalRequest) -> Self {
        Requested {
            request: request.id,
            action: request.action.clone(),
            subject: request.subject(),
        }
    }
}

impl Notification for Requested {
    fn kind(&self) -> &'static str {
        REQUESTED_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let lang = state.current_lang();
        let title = texts::t_with(
            &lang,
            "wl_kernel.approvals.requested_title",
            &[("action", &self.action)],
        );
        let body = texts::t_with(
            &lang,
            "wl_kernel.approvals.requested_body",
            &[("request", &self.request), ("subject", &self.subject)],
        );
        Ok(DatabaseMessage::info(title)
            .body(body)
            .with("approval_request", self.request)
            .with("subject_type", &self.subject.subject_type)
            .with("subject_id", self.subject.subject_id)
            .into())
    }
}

#[cfg(test)]
mod tests {
    use renox::db::Json;

    use super::*;

    #[test]
    fn the_notice_names_the_request_its_action_and_subject() {
        let request = ApprovalRequest {
            id: 5,
            subject_type: "payments.refund".into(),
            subject_id: 9,
            action: "payments.refund".into(),
            steps: Json(Vec::new()),
            ..Default::default()
        };
        let notice = Requested::of(&request);
        assert_eq!(notice.request, 5);
        assert_eq!(notice.action, "payments.refund");
        assert_eq!(notice.subject, Subject::new("payments.refund", 9));
        assert_eq!(notice.kind(), REQUESTED_KIND);
        assert_eq!(
            notice.channels(&Recipient::default()),
            vec![Channel::Database]
        );
    }
}
