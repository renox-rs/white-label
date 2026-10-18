//! Who decides a request, and the notices the kernel sends about it
//! (wl-kernel §4.1 effects).
//!
//! wl-notify is not built yet, so the kernel sends Renox notifications to
//! users directly, on the database channel (the in-app bell), after the
//! commit. Approvers are the holders of one of the current step's roles, or
//! of its permission, in the subject's scope (from its registered machine),
//! or globally when neither a machine nor a resolver gives a scope
//! ([`crate::scopes`]).

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::Value;

use super::end::{End, SubjectEnd};
use super::events::ApprovalEnd;
use super::request::ApprovalRequest;
use crate::scopes;
use crate::subject::Subject;
use crate::texts;

/// The kind of the notice to approvers that a request waits for them.
pub const REQUESTED_KIND: &str = "approval-requested";
/// The kind of the notice that a request ended without a yes.
pub const DECIDED_KIND: &str = "approval-decided";

/// The scope approvers of `request` are looked for in: the subject's, from
/// [`scopes::scope_of`] (its machine, then a registered resolver); global
/// when it has neither.
pub async fn scope_of(db: &Db, request: &ApprovalRequest) -> Result<Scope> {
    scopes::scope_of(db, &request.subject()).await
}

/// The users who may decide the request's current step: holders of one of
/// its roles, or of its permission, in the subject's scope, ordered by id. With SoD on, the
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
    let mut holders = Vec::new();
    for role in &step.roles {
        holders.extend(users_with_role_in(db, role, &scope).await?);
    }
    if let Some(permission) = &step.permission {
        holders.extend(users_with_permission_in(db, permission, &scope).await?);
    }
    let mut seen = BTreeSet::new();
    let mut approvers = Vec::new();
    for user in holders {
        if !excluded.contains(&user.id) && seen.insert(user.id) {
            approvers.push(user);
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

/// Who is told about an end that is not a yes: the requester on `reject`,
/// `return` and `expire`; the requester and the last approver on `fail`; the current
/// step's approvers on `withdraw` (`step_approvers`), and the requester too
/// when the subject had already moved and someone else ended it. Sorted,
/// without repeats.
fn decided_recipients(
    request: &ApprovalRequest,
    end: &End,
    moved: bool,
    step_approvers: &[i64],
) -> Vec<i64> {
    let mut to = BTreeSet::new();
    match end.kind() {
        ApprovalEnd::Rejected | ApprovalEnd::Returned | ApprovalEnd::Expired => {
            to.insert(request.requester_id);
        }
        ApprovalEnd::Failed => {
            to.insert(request.requester_id);
            to.extend(end.actor_id());
        }
        ApprovalEnd::Withdrawn => to.extend(step_approvers.iter().copied()),
        ApprovalEnd::Approved => {}
    }
    if moved && end.actor_id() != Some(request.requester_id) {
        to.insert(request.requester_id);
    }
    to.into_iter().collect()
}

/// Tells the people an end that is not a yes concerns (see
/// `decided_recipients`), with the reason and, when the subject had already
/// moved, where it is. Call it after the commit.
pub async fn decided(
    state: &AppState,
    request: &ApprovalRequest,
    end: &End,
    subject: &SubjectEnd,
) -> Result {
    let step_approvers: Vec<i64> = if end.kind() == ApprovalEnd::Withdrawn {
        step_approvers(&state.db, request)
            .await?
            .iter()
            .map(|user| user.id)
            .collect()
    } else {
        Vec::new()
    };
    let notice = Decided::of(request, end, subject);
    for id in decided_recipients(request, end, subject.moved_to.is_some(), &step_approvers) {
        if let Some(user) = User::find(&state.db, id).await? {
            state.notify(&user, &notice).await?;
        }
    }
    Ok(())
}

/// Tells the requester that `request` was approved and its action ran.
/// Call it after the commit.
pub async fn approved(state: &AppState, request: &ApprovalRequest) -> Result {
    let notice = Decided::approved(request);
    if let Some(user) = User::find(&state.db, request.requester_id).await? {
        state.notify(&user, &notice).await?;
    }
    Ok(())
}

/// The notice that a request ended: without a yes, or approved
/// ([`Decided::approved`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// The request's id.
    pub request: i64,
    /// The action key.
    pub action: String,
    /// The record the request is about.
    pub subject: Subject,
    /// How it ended.
    pub decision: ApprovalEnd,
    /// The approver's comment, or the fixed reason.
    pub reason: String,
    /// Where the subject was found, when it had already moved.
    pub moved_to: Option<String>,
}

impl Decided {
    /// The notice about `request` ending with `end`.
    pub fn of(request: &ApprovalRequest, end: &End, subject: &SubjectEnd) -> Self {
        Decided {
            request: request.id,
            action: request.action.clone(),
            subject: request.subject(),
            decision: end.kind(),
            reason: end.reason().to_owned(),
            moved_to: subject.moved_to.clone(),
        }
    }

    /// The notice that `request` was approved.
    pub fn approved(request: &ApprovalRequest) -> Self {
        Decided {
            request: request.id,
            action: request.action.clone(),
            subject: request.subject(),
            decision: ApprovalEnd::Approved,
            reason: String::new(),
            moved_to: None,
        }
    }

    /// The key of the notice's title.
    fn title_key(&self) -> &'static str {
        match self.decision {
            ApprovalEnd::Rejected => "wl_kernel.approvals.rejected_title",
            ApprovalEnd::Withdrawn => "wl_kernel.approvals.withdrawn_title",
            ApprovalEnd::Expired => "wl_kernel.approvals.expired_title",
            ApprovalEnd::Failed => "wl_kernel.approvals.failed_title",
            ApprovalEnd::Approved => "wl_kernel.approvals.approved_title",
            ApprovalEnd::Returned => "wl_kernel.approvals.returned_title",
        }
    }

    /// The notice's title and body in `lang`.
    fn texts(&self, lang: &Lang) -> (String, String) {
        let title = texts::t_with(lang, self.title_key(), &[("action", &self.action)]);
        if self.decision == ApprovalEnd::Approved {
            let body = texts::t_with(
                lang,
                "wl_kernel.approvals.approved_body",
                &[("request", &self.request), ("subject", &self.subject)],
            );
            return (title, body);
        }
        let mut body = texts::t_with(
            lang,
            "wl_kernel.approvals.decided_body",
            &[
                ("request", &self.request),
                ("subject", &self.subject),
                ("reason", &self.reason),
            ],
        );
        if let Some(state) = &self.moved_to {
            body.push(' ');
            body.push_str(&texts::t_with(
                lang,
                "wl_kernel.approvals.subject_moved",
                &[("state", state)],
            ));
        }
        (title, body)
    }
}

impl Notification for Decided {
    fn kind(&self) -> &'static str {
        DECIDED_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let (title, body) = self.texts(&state.current_lang());
        let message = match self.decision {
            ApprovalEnd::Rejected | ApprovalEnd::Failed => DatabaseMessage::error(title),
            _ => DatabaseMessage::info(title),
        };
        Ok(message
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

    fn requested_by(requester_id: i64) -> ApprovalRequest {
        ApprovalRequest {
            id: 5,
            subject_type: "payments.refund".into(),
            subject_id: 9,
            action: "payments.refund".into(),
            requester_id,
            steps: Json(Vec::new()),
            ..Default::default()
        }
    }

    #[test]
    fn the_requester_hears_of_a_reject_or_an_expiry() {
        let request = requested_by(1);
        let reject = End::reject(2, 1, "duplicate");
        assert_eq!(decided_recipients(&request, &reject, false, &[]), vec![1]);
        assert_eq!(
            decided_recipients(&request, &End::expire(), false, &[]),
            vec![1]
        );
    }

    #[test]
    fn the_requester_hears_of_a_return_with_the_comment() {
        let request = requested_by(1);
        let end = End::return_for_changes(2, 1, "attach the photos");
        assert_eq!(decided_recipients(&request, &end, false, &[]), vec![1]);
        let notice = Decided::of(&request, &end, &SubjectEnd::default());
        assert_eq!(notice.decision, ApprovalEnd::Returned);
        assert_eq!(notice.reason, "attach the photos");
        assert_eq!(
            texts::en_with(notice.title_key(), &[("action", &notice.action)]),
            "Returned for changes: fix and resubmit (payments.refund)"
        );
    }

    #[test]
    fn a_failure_reaches_the_requester_and_the_last_approver() {
        let request = requested_by(1);
        let fail = End::fail(Some(3), "over the balance");
        assert_eq!(decided_recipients(&request, &fail, false, &[]), vec![1, 3]);
    }

    #[test]
    fn a_withdrawal_reaches_the_approvers_only() {
        let request = requested_by(1);
        let withdraw = End::withdraw(1);
        assert_eq!(
            decided_recipients(&request, &withdraw, false, &[4, 2, 4]),
            vec![2, 4]
        );
        assert_eq!(decided_recipients(&request, &withdraw, true, &[2]), vec![2]);
    }

    #[test]
    fn the_notice_gives_the_reason_and_where_the_subject_had_moved() {
        let request = requested_by(1);
        let moved = SubjectEnd {
            returned_to: None,
            moved_to: Some("cancelled".into()),
        };
        let notice = Decided::of(&request, &End::reject(2, 1, "duplicate"), &moved);
        assert_eq!(notice.kind(), DECIDED_KIND);
        assert_eq!(notice.decision, ApprovalEnd::Rejected);
        assert_eq!(notice.reason, "duplicate");
        assert_eq!(notice.moved_to.as_deref(), Some("cancelled"));
        assert_eq!(
            texts::en_with(notice.title_key(), &[("action", &notice.action)]),
            "Approval rejected: payments.refund"
        );
        assert_eq!(
            texts::en_with(
                "wl_kernel.approvals.subject_moved",
                &[("state", &"cancelled")]
            ),
            "Subject already moved to `cancelled`."
        );
    }

    #[test]
    fn the_approved_notice_says_the_request_was_approved() {
        let notice = Decided::approved(&requested_by(1));
        assert_eq!(notice.decision, ApprovalEnd::Approved);
        assert!(notice.reason.is_empty() && notice.moved_to.is_none());
        assert_eq!(
            texts::en_with(notice.title_key(), &[("action", &notice.action)]),
            "Approval granted: payments.refund"
        );
        assert_eq!(
            texts::en_with(
                "wl_kernel.approvals.approved_body",
                &[
                    ("request", &notice.request),
                    ("subject", &"payments.refund #9")
                ]
            ),
            "Request #5 about payments.refund #9 was approved."
        );
    }
}
