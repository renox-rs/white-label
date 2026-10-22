//! The back office's part of approvals (wl-kernel §4.1, `PLT.APR.01`): the
//! approval card a record's page shows, the routes its buttons post to, the
//! pending-approval badge, and the request's items for the record's Activity
//! tab.
//!
//! A module's record page gives its template `approvals` ([`cards_for`]) and
//! includes the card:
//!
//! ```html
//! <rx-include template="wl-kernel/approval_card.html"/>
//! ```
//!
//! Each card shows a pending request: its status, steps and decisions, and
//! the buttons this viewer may use: approve and reject (comment required)
//! for an approver of the current step, withdraw for the requester. The
//! buttons post to [`routes`] (mounted by the kernel). The badge is
//! [`pending_for`]: the data says whether a subject waits, so modules keep no
//! column for it. [`timeline_for`] gives the request and its decisions as
//! items in the shape of `renox::state_machine::timeline`, for the module to
//! merge with the subject's own history.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};

use super::decide;
use super::end;
use super::events::ApprovalEnd;
use super::policy::Step;
use super::request::{ApprovalDecision, ApprovalRequest, ApprovalStatus, DecisionKind};
use super::resubmit::{self, EarlierDecision};
use super::tasks::{self, ResubmitTask};
use crate::calendar::BusinessCalendar;
use crate::subject::Subject;
use crate::texts;

/// The template of the card, compiled into the kernel.
pub const CARD_TEMPLATE: &str = "wl-kernel/approval_card.html";

/// The names of the decision routes.
pub mod route_names {
    /// `POST /approvals/{id}/approve`.
    pub const APPROVE: &str = "wl_kernel.approvals.approve";
    /// `POST /approvals/{id}/reject`.
    pub const REJECT: &str = "wl_kernel.approvals.reject";
    /// `POST /approvals/{id}/withdraw`.
    pub const WITHDRAW: &str = "wl_kernel.approvals.withdraw";
    /// `POST /approvals/{id}/return`.
    pub const RETURN: &str = "wl_kernel.approvals.return";
    /// `POST /approvals/{id}/not-resubmitting`.
    pub const NOT_RESUBMITTING: &str = "wl_kernel.approvals.not_resubmitting";
}

/// The pending requests about `subject`, oldest first: the pending-approval
/// badge shows while this is not empty.
pub async fn pending_for(db: &Db, subject: &Subject) -> Result<Vec<ApprovalRequest>> {
    ApprovalRequest::pending_about(db, subject).await
}

/// One step of a request, as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardStep {
    /// The step, from 1.
    pub number: i64,
    /// Who may decide it: its roles, and its permission's holders.
    pub approvers: String,
    /// `done`, `current`, `waiting` or `closed`.
    pub state: &'static str,
    /// The state, in words.
    pub state_label: String,
    /// The badge's colour.
    pub kind: &'static str,
}

/// One decision taken on a request, as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardDecision {
    /// The step decided.
    pub step: i64,
    /// Who decided, by name.
    pub approver: String,
    /// `approve`, `reject` or `return`.
    pub decision: String,
    /// The decision, in words.
    pub decision_label: String,
    /// The badge's colour.
    pub kind: &'static str,
    /// The approver's comment.
    pub comment: Option<String>,
    /// When it was taken.
    pub at: Option<DateTime>,
}

/// One earlier return or rejection in the chain before a request, as the
/// card shows it: "rejected on 2026-11-02 by Bilal: duplicate".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardEarlier {
    /// `returned` or `rejected`.
    pub status: String,
    /// The badge's colour.
    pub kind: &'static str,
    /// The whole line, in the viewer's language.
    pub text: String,
}

/// What the viewer may do on a card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CardRights {
    /// Approve, reject or return the current step.
    decide: bool,
    /// Withdraw the request (the requester).
    withdraw: bool,
    /// Say "not resubmitting" (the requester of a returned request whose
    /// task is open).
    close_task: bool,
    /// The returned request's task is still open.
    task_open: bool,
}

/// What the approval card shows about one request, for one viewer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalCard {
    /// The request's id (the routes take it).
    pub id: i64,
    /// The card's heading.
    pub title: String,
    /// The action key, e.g. `payments.refund`.
    pub action: String,
    /// The request's state, e.g. `pending`.
    pub status: String,
    /// The state, in words.
    pub status_label: String,
    /// The status badge's colour.
    pub status_kind: &'static str,
    /// The requester, by name.
    pub requester: String,
    /// When it was requested.
    pub created_at: Option<DateTime>,
    /// When it expires (an instant, shown in `APP_TIMEZONE`, Asia/Dubai).
    pub expires_at: Option<DateTime>,
    /// "Step 1 of 2".
    pub step_label: String,
    /// Every step of the request.
    pub steps: Vec<CardStep>,
    /// The decisions taken, in order.
    pub decisions: Vec<CardDecision>,
    /// "Subject already moved to `<state>`", when it did.
    pub subject_moved: Option<String>,
    /// Whether the viewer may approve or reject the current step.
    pub can_decide: bool,
    /// Whether the viewer may withdraw the request (the requester).
    pub can_withdraw: bool,
    /// Whether the viewer may return the request for changes (an approver of
    /// the current step, like `can_decide`).
    pub can_return: bool,
    /// Whether the request is returned and its "fix and resubmit" task is
    /// still open.
    pub task_open: bool,
    /// Whether the viewer may say "not resubmitting" (the requester).
    pub can_close_task: bool,
    /// The returns and rejections before this request, newest first.
    pub earlier: Vec<CardEarlier>,
    /// The card's texts, in the viewer's language, by name.
    pub labels: BTreeMap<&'static str, String>,
}

/// Where texts come from: the viewer's language, or English (`None`, for
/// what is built without a request).
#[derive(Clone, Copy)]
struct Tr<'a>(Option<&'a Lang>);

impl Tr<'_> {
    fn t(self, key: &str) -> String {
        self.t_with(key, &[])
    }

    fn t_with(self, key: &str, params: &[(&str, &dyn Display)]) -> String {
        match self.0 {
            Some(lang) => texts::t_with(lang, key, params),
            None => texts::en_with(key, params),
        }
    }
}

/// The card's texts: the name the template uses, and the key.
const LABELS: &[(&str, &str)] = &[
    ("status", "wl_kernel.approvals.card.status"),
    ("requester", "wl_kernel.approvals.card.requester"),
    ("requested_at", "wl_kernel.approvals.card.requested_at"),
    ("expires_at", "wl_kernel.approvals.card.expires_at"),
    ("current_step", "wl_kernel.approvals.card.current_step"),
    ("steps", "wl_kernel.approvals.card.steps"),
    ("step", "wl_kernel.approvals.card.step"),
    ("approvers", "wl_kernel.approvals.card.approvers"),
    ("decisions", "wl_kernel.approvals.card.decisions"),
    ("decision", "wl_kernel.approvals.card.decision"),
    ("decided_by", "wl_kernel.approvals.card.decided_by"),
    ("decided_at", "wl_kernel.approvals.card.decided_at"),
    ("comment", "wl_kernel.approvals.card.comment"),
    (
        "comment_optional",
        "wl_kernel.approvals.card.comment_optional",
    ),
    ("comment_reject", "wl_kernel.approvals.card.comment_reject"),
    ("approve", "wl_kernel.approvals.card.approve"),
    ("approve_title", "wl_kernel.approvals.card.approve_title"),
    ("reject", "wl_kernel.approvals.card.reject"),
    ("reject_title", "wl_kernel.approvals.card.reject_title"),
    ("withdraw", "wl_kernel.approvals.card.withdraw"),
    ("withdraw_title", "wl_kernel.approvals.card.withdraw_title"),
    (
        "withdraw_message",
        "wl_kernel.approvals.card.withdraw_message",
    ),
    ("cancel", "wl_kernel.approvals.card.cancel"),
    ("earlier", "wl_kernel.approvals.card.earlier"),
    ("return", "wl_kernel.approvals.card.return"),
    ("return_title", "wl_kernel.approvals.card.return_title"),
    ("comment_return", "wl_kernel.approvals.card.comment_return"),
    ("task_open", "wl_kernel.approvals.card.task_open"),
    (
        "not_resubmitting",
        "wl_kernel.approvals.card.not_resubmitting",
    ),
    (
        "not_resubmitting_title",
        "wl_kernel.approvals.card.not_resubmitting_title",
    ),
    (
        "not_resubmitting_message",
        "wl_kernel.approvals.card.not_resubmitting_message",
    ),
];

fn labels(tr: Tr<'_>) -> BTreeMap<&'static str, String> {
    LABELS
        .iter()
        .map(|(name, key)| (*name, tr.t(key)))
        .collect()
}

/// The badge colour of a request's state (the machine's tones; `neutral`
/// is the kit's `info`).
pub fn status_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending | ApprovalStatus::Returned => "warning",
        ApprovalStatus::Approved => "success",
        ApprovalStatus::Rejected | ApprovalStatus::Failed => "error",
        ApprovalStatus::Withdrawn | ApprovalStatus::Expired => "info",
    }
}

fn status_label(tr: Tr<'_>, status: ApprovalStatus) -> String {
    tr.t(&format!("wl_kernel.approvals.status.{status}"))
}

fn decision_kind(decision: DecisionKind) -> &'static str {
    match decision {
        DecisionKind::Approve => "success",
        DecisionKind::Reject => "error",
        DecisionKind::Return => "warning",
    }
}

fn decision_label(tr: Tr<'_>, decision: DecisionKind) -> String {
    tr.t(&format!("wl_kernel.approvals.decision.{decision}"))
}

/// Who may decide `step`, in words: its roles, then its permission's holders.
fn step_approvers(tr: Tr<'_>, step: &Step) -> String {
    let mut parts = step.roles.clone();
    if let Some(permission) = &step.permission {
        parts.push(tr.t_with(
            "wl_kernel.approvals.card.holders_of",
            &[("permission", permission)],
        ));
    }
    parts.join(", ")
}

/// Where step `number` stands on `request`: `done` (approved), `current`
/// (waiting for a decision), `waiting` (not reached yet) or `closed` (the
/// request ended without reaching or approving it).
fn step_state(number: i64, request: &ApprovalRequest) -> &'static str {
    let current = request.current_step;
    if number < current || request.status == ApprovalStatus::Approved {
        "done"
    } else if !request.is_pending() {
        "closed"
    } else if number == current {
        "current"
    } else {
        "waiting"
    }
}

fn step_kind(state: &str) -> &'static str {
    match state {
        "done" => "success",
        "current" => "warning",
        _ => "info",
    }
}

/// The names of users `ids`; a missing user reads "User #<id>".
async fn names(db: &Db, ids: impl IntoIterator<Item = i64>) -> Result<BTreeMap<i64, String>> {
    let ids: Vec<i64> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = User::query().where_in("id", ids).get(db).await?;
    Ok(users.into_iter().map(|user| (user.id, user.name)).collect())
}

fn name_of(tr: Tr<'_>, names: &BTreeMap<i64, String>, id: i64) -> String {
    names
        .get(&id)
        .cloned()
        .unwrap_or_else(|| tr.t_with("wl_kernel.approvals.card.unknown_user", &[("id", &id)]))
}

/// The card of `request` for `viewer`, in `lang`.
pub async fn card(
    db: &Db,
    request: &ApprovalRequest,
    viewer: &User,
    lang: &Lang,
) -> Result<ApprovalCard> {
    let decisions = request.decisions(db).await?;
    let earlier = resubmit::earlier_decisions(db, request).await?;
    let people = names(
        db,
        std::iter::once(request.requester_id)
            .chain(decisions.iter().map(|d| d.approver_id))
            .chain(earlier.iter().map(|e| e.approver)),
    )
    .await?;
    let pending = request.is_pending();
    let task_open = request.status == ApprovalStatus::Returned
        && ResubmitTask::open_for(db, request.id).await?.is_some();
    let rights = CardRights {
        decide: pending && decide::ensure_approver(db, request, viewer).await.is_ok(),
        withdraw: pending && request.requester_id == viewer.id,
        close_task: task_open && request.requester_id == viewer.id,
        task_open,
    };
    Ok(build_card(
        request,
        &decisions,
        &earlier,
        &people,
        rights,
        Tr(Some(lang)),
    ))
}

/// The line of one earlier decision: "rejected on 2026-11-02 by Bilal:
/// duplicate". The date is the Asia/Dubai day.
fn earlier_line(tr: Tr<'_>, earlier: &EarlierDecision, people: &BTreeMap<i64, String>) -> String {
    let key = match earlier.status {
        ApprovalStatus::Returned => "wl_kernel.approvals.card.earlier_returned",
        _ => "wl_kernel.approvals.card.earlier_rejected",
    };
    let date = earlier.at.map_or_else(
        || "—".to_owned(),
        |at| {
            at.with_timezone(&BusinessCalendar::offset())
                .format("%Y-%m-%d")
                .to_string()
        },
    );
    let line = tr.t_with(
        key,
        &[
            ("date", &date),
            ("name", &name_of(tr, people, earlier.approver)),
        ],
    );
    match earlier.comment.trim() {
        "" => line,
        comment => format!("{line}: {comment}"),
    }
}

/// The card, from what [`card`] loaded.
fn build_card(
    request: &ApprovalRequest,
    decisions: &[ApprovalDecision],
    earlier: &[EarlierDecision],
    people: &BTreeMap<i64, String>,
    rights: CardRights,
    tr: Tr<'_>,
) -> ApprovalCard {
    let steps = request
        .steps
        .iter()
        .zip(1_i64..)
        .map(|(step, number)| {
            let state = step_state(number, request);
            CardStep {
                number,
                approvers: step_approvers(tr, step),
                state,
                state_label: tr.t(&format!("wl_kernel.approvals.step_state.{state}")),
                kind: step_kind(state),
            }
        })
        .collect();
    let decisions = decisions
        .iter()
        .map(|decision| CardDecision {
            step: decision.step,
            approver: name_of(tr, people, decision.approver_id),
            decision: decision.decision.to_string(),
            decision_label: decision_label(tr, decision.decision),
            kind: decision_kind(decision.decision),
            comment: decision.comment.clone().filter(|c| !c.trim().is_empty()),
            at: decision.created_at,
        })
        .collect();
    ApprovalCard {
        id: request.id,
        title: tr.t_with(
            "wl_kernel.approvals.card.title",
            &[(
                "action",
                &match tr.0 {
                    Some(lang) => super::awaiting::action_label(lang, &request.action),
                    None => super::awaiting::platform_label(&request.action)
                        .map_or_else(|| request.action.clone(), str::to_owned),
                },
            )],
        ),
        action: request.action.clone(),
        status: request.status.to_string(),
        status_label: status_label(tr, request.status),
        status_kind: status_kind(request.status),
        requester: name_of(tr, people, request.requester_id),
        created_at: request.created_at,
        expires_at: request.expires_at,
        step_label: tr.t_with(
            "wl_kernel.approvals.card.step_of",
            &[
                ("step", &request.current_step),
                ("steps", &request.steps.len()),
            ],
        ),
        steps,
        decisions,
        subject_moved: request
            .subject_moved_to
            .as_ref()
            .map(|state| tr.t_with("wl_kernel.approvals.subject_moved", &[("state", state)])),
        can_decide: rights.decide,
        can_withdraw: rights.withdraw,
        can_return: rights.decide,
        task_open: rights.task_open,
        can_close_task: rights.close_task,
        earlier: earlier
            .iter()
            .map(|e| CardEarlier {
                status: e.status.to_string(),
                kind: status_kind(e.status),
                text: earlier_line(tr, e, people),
            })
            .collect(),
        labels: labels(tr),
    }
}

/// The cards of the pending requests about `subject`, and of the returned
/// ones still waiting for their requester (an open "fix and resubmit"
/// task), for `viewer`: what a record's page gives the card template as
/// `approvals`.
pub async fn cards_for(
    db: &Db,
    subject: &Subject,
    viewer: &User,
    lang: &Lang,
) -> Result<Vec<ApprovalCard>> {
    let mut cards = Vec::new();
    for request in pending_for(db, subject).await? {
        cards.push(card(db, &request, viewer, lang).await?);
    }
    let returned = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Returned))
        .order_by(ApprovalRequest::ID)
        .get(db)
        .await?;
    for request in returned {
        if ResubmitTask::open_for(db, request.id).await?.is_some() {
            cards.push(card(db, &request, viewer, lang).await?);
        }
    }
    Ok(cards)
}

/// One thing the user must do: fix a returned request and resubmit it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedsActionItem {
    /// The returned request.
    pub request_id: i64,
    /// What to do, in the viewer's language.
    pub title: String,
    /// The approver's comment on the return.
    pub comment: Option<String>,
    /// The record the request is about: its type (e.g. `invoicing.invoice`).
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// When it was returned.
    pub returned_at: Option<DateTime>,
}

/// The dashboard's "Needs your action" items for `user`, newest first: one
/// per open "fix and resubmit" task ([`tasks::needs_action`]).
pub async fn needs_action_items(db: &Db, user: &User, lang: &Lang) -> Result<Vec<NeedsActionItem>> {
    let mut items = Vec::new();
    for task in tasks::needs_action(db, user).await? {
        let Some(request) = ApprovalRequest::find(db, task.request_id).await? else {
            continue;
        };
        let comment = ApprovalDecision::query()
            .where_(ApprovalDecision::REQUEST_ID.eq(request.id))
            .where_(ApprovalDecision::DECISION.eq(DecisionKind::Return))
            .order_by_desc(ApprovalDecision::ID)
            .first(db)
            .await?
            .and_then(|d| d.comment)
            .filter(|c| !c.trim().is_empty());
        items.push(NeedsActionItem {
            request_id: request.id,
            title: texts::t_with(
                lang,
                "wl_kernel.approvals.needs_action.item",
                &[(
                    "action",
                    &super::awaiting::action_label(lang, &request.action),
                )],
            ),
            comment,
            subject_type: request.subject_type.clone(),
            subject_id: request.subject_id,
            returned_at: request.decided_at.or(task.created_at),
        });
    }
    Ok(items)
}

/// The timeline items of every request about `subject` (pending or ended),
/// oldest first, in the shape `renox::state_machine::timeline` gives
/// (`time`, `title`, `body`, `kind`): the request, each decision, and the
/// end with its reason. A module merges them with the subject's own history
/// for its Activity tab.
pub async fn timeline_for(db: &Db, subject: &Subject, lang: &Lang) -> Result<Vec<Value>> {
    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(subject.subject_id))
        .order_by(ApprovalRequest::ID)
        .get(db)
        .await?;
    let mut items = Vec::new();
    for request in &requests {
        let decisions = request.decisions(db).await?;
        let history = renox::state_machine::history(db, ApprovalRequest::TABLE, request.id).await?;
        // The reason of the row that ended the request.
        let reason = history
            .iter()
            .rev()
            .find(|row| row.to_state == request.status.as_str())
            .and_then(|row| row.reason.as_deref());
        let people = names(
            db,
            std::iter::once(request.requester_id).chain(decisions.iter().map(|d| d.approver_id)),
        )
        .await?;
        items.extend(request_items(
            request,
            &decisions,
            reason,
            &people,
            Tr(Some(lang)),
        ));
    }
    items.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));
    Ok(items)
}

fn time(at: Option<DateTime>) -> Value {
    at.map_or(Value::Null, |at| Value::String(at.to_rfc3339()))
}

/// The items of one request: requested, each decision, and its end with
/// `end_reason` (the history reason of the row that ended it).
fn request_items(
    request: &ApprovalRequest,
    decisions: &[ApprovalDecision],
    end_reason: Option<&str>,
    people: &BTreeMap<i64, String>,
    tr: Tr<'_>,
) -> Vec<Value> {
    let mut items = vec![json!({
        "time": time(request.created_at),
        "title": tr.t_with(
            "wl_kernel.approvals.timeline.requested",
            &[("action", &request.action)],
        ),
        "body": tr.t_with(
            "wl_kernel.approvals.timeline.requested_by",
            &[("name", &name_of(tr, people, request.requester_id))],
        ),
        "kind": "warning",
    })];
    for decision in decisions {
        items.push(json!({
            "time": time(decision.created_at),
            "title": tr.t_with(
                "wl_kernel.approvals.timeline.decision",
                &[
                    ("decision", &decision_label(tr, decision.decision)),
                    ("step", &decision.step),
                    ("name", &name_of(tr, people, decision.approver_id)),
                ],
            ),
            "body": decision.comment.clone().filter(|c| !c.trim().is_empty()),
            "kind": decision_kind(decision.decision),
        }));
    }
    if let Some(end) = request.status.end() {
        let reason = end_reason.map(str::to_owned);
        let moved = request
            .subject_moved_to
            .as_ref()
            .map(|state| tr.t_with("wl_kernel.approvals.subject_moved", &[("state", state)]));
        let body: Vec<String> = reason.into_iter().chain(moved).collect();
        items.push(json!({
            "time": time(request.decided_at.or(request.updated_at)),
            "title": tr.t_with(end_title_key(end), &[("action", &request.action)]),
            "body": if body.is_empty() { Value::Null } else { Value::String(body.join(" ")) },
            "kind": status_kind(request.status),
        }));
    }
    items
}

fn end_title_key(end: ApprovalEnd) -> &'static str {
    match end {
        ApprovalEnd::Approved => "wl_kernel.approvals.approved_title",
        ApprovalEnd::Rejected => "wl_kernel.approvals.rejected_title",
        ApprovalEnd::Returned => "wl_kernel.approvals.returned_title",
        ApprovalEnd::Withdrawn => "wl_kernel.approvals.withdrawn_title",
        ApprovalEnd::Expired => "wl_kernel.approvals.expired_title",
        ApprovalEnd::Failed => "wl_kernel.approvals.failed_title",
    }
}

/// The decision routes the card's buttons post to, for logged-in users;
/// who may decide is checked per request (403 otherwise):
///
/// - `POST /approvals/{id}/approve` (optional `comment`), an approver of
///   the current step;
/// - `POST /approvals/{id}/reject` (`comment` required, not blank), the
///   same;
/// - `POST /approvals/{id}/return` (`comment` required, not blank), the
///   same: sends the request back for changes;
/// - `POST /approvals/{id}/withdraw`, the requester;
/// - `POST /approvals/{id}/not-resubmitting`, the requester of a returned
///   request: closes its "fix and resubmit" task (422 when already closed).
pub fn routes() -> Routes {
    Routes::new()
        .post("/approvals/{id}/approve", approve)
        .name(route_names::APPROVE)
        .post("/approvals/{id}/reject", reject)
        .name(route_names::REJECT)
        .post("/approvals/{id}/withdraw", withdraw)
        .name(route_names::WITHDRAW)
        .post("/approvals/{id}/return", return_for_changes)
        .name(route_names::RETURN)
        .post("/approvals/{id}/not-resubmitting", not_resubmitting)
        .name(route_names::NOT_RESUBMITTING)
        .require_auth()
}

/// The form of approve and reject.
#[derive(Debug, Default, Deserialize, Serialize)]
struct DecisionForm {
    #[serde(default)]
    comment: Option<String>,
}

impl Validate for DecisionForm {
    fn rules(&self, _v: &mut Validator) {}
}

impl DecisionForm {
    fn comment(&self) -> &str {
        self.comment.as_deref().map_or("", str::trim)
    }
}

/// The 422 under the comment field when a reject has none.
fn comment_required(tr: Tr<'_>, form: &DecisionForm) -> Error {
    let mut errors = Errors::new();
    errors.add("comment", tr.t("wl_kernel.approvals.comment_required"));
    ValidationError::new(errors).with_input(form).into()
}

/// The toast after a yes: the request approved, failed, or waiting for its
/// next step.
fn approved_toast(tr: Tr<'_>, end: Option<ApprovalEnd>) -> Toast {
    match end {
        Some(ApprovalEnd::Approved) => Toast::success(tr.t("wl_kernel.approvals.toast.approved")),
        Some(_) => Toast::error(tr.t("wl_kernel.approvals.toast.failed")),
        None => Toast::success(tr.t("wl_kernel.approvals.toast.step_approved")),
    }
}

async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
    Valid(form): Valid<DecisionForm>,
) -> Result<(Toast, HxRefresh)> {
    let decided = decide::approve(&state, id, &user, form.comment()).await?;
    Ok((
        approved_toast(Tr(Some(&lang)), decided.map(|d| d.decision)),
        HxRefresh,
    ))
}

async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
    Valid(form): Valid<DecisionForm>,
) -> Result<(Toast, HxRefresh)> {
    if form.comment().is_empty() {
        return Err(comment_required(Tr(Some(&lang)), &form));
    }
    end::reject(&state, id, &user, form.comment()).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_kernel.approvals.toast.rejected")),
        HxRefresh,
    ))
}

async fn return_for_changes(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
    Valid(form): Valid<DecisionForm>,
) -> Result<(Toast, HxRefresh)> {
    if form.comment().is_empty() {
        return Err(comment_required(Tr(Some(&lang)), &form));
    }
    end::return_request(&state, id, &user, form.comment()).await?;
    Ok((
        Toast::success(texts::t(&lang, "wl_kernel.approvals.toast.returned")),
        HxRefresh,
    ))
}

async fn not_resubmitting(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
) -> Result<(Toast, HxRefresh)> {
    tasks::close_not_resubmitting(&state.db, id, &user).await?;
    Ok((
        Toast::success(texts::t(
            &lang,
            "wl_kernel.approvals.toast.not_resubmitting",
        )),
        HxRefresh,
    ))
}

async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    lang: Lang,
    back: Back,
) -> Result<Response> {
    end::withdraw(&state, id, &user).await?;
    let toast = Toast::success(texts::t(&lang, "wl_kernel.approvals.toast.withdrawn"));
    Ok((toast, back).into_response())
}

#[cfg(test)]
mod tests {
    use renox::ToastKind;
    use renox::chrono::{TimeZone, Utc};
    use renox::db::Json;

    use super::*;

    fn at(minute: u32) -> Option<DateTime> {
        Some(Utc.with_ymd_and_hms(2026, 11, 3, 10, minute, 0).unwrap())
    }

    fn request(status: ApprovalStatus, current_step: i64) -> ApprovalRequest {
        ApprovalRequest {
            id: 7,
            subject_type: "payments.refund".into(),
            subject_id: 3,
            action: "payments.refund".into(),
            transition: "refund".into(),
            requester_id: 1,
            current_step,
            steps: Json(vec![
                Step::one_of(["property_manager"]),
                Step::one_of(["finance", "owner"]).or_permission("refunds.approve"),
            ]),
            status,
            created_at: at(0),
            decided_at: at(30),
            ..Default::default()
        }
    }

    fn people() -> BTreeMap<i64, String> {
        BTreeMap::from([(1, "Aisha".to_owned()), (2, "Bilal".to_owned())])
    }

    fn approved_step_one() -> ApprovalDecision {
        ApprovalDecision {
            id: 1,
            request_id: 7,
            step: 1,
            approver_id: 2,
            decision: DecisionKind::Approve,
            comment: Some("  ".into()),
            created_at: at(10),
            ..Default::default()
        }
    }

    #[test]
    fn a_pending_card_shows_its_steps_decisions_and_the_viewers_buttons() {
        let request = request(ApprovalStatus::Pending, 2);
        let card = build_card(
            &request,
            &[approved_step_one()],
            &[],
            &people(),
            CardRights {
                decide: true,
                ..Default::default()
            },
            Tr(None),
        );
        assert_eq!(card.title, "Approval: Pay a refund");
        assert_eq!(card.status, "pending");
        assert_eq!(card.status_label, "Pending approval");
        assert_eq!(card.status_kind, "warning");
        assert_eq!(card.requester, "Aisha");
        assert_eq!(card.step_label, "Step 2 of 2");
        let states: Vec<_> = card.steps.iter().map(|s| s.state).collect();
        assert_eq!(states, ["done", "current"]);
        assert_eq!(card.steps[0].approvers, "property_manager");
        assert_eq!(
            card.steps[1].approvers,
            "finance, owner, holders of refunds.approve"
        );
        assert_eq!(card.decisions.len(), 1);
        assert_eq!(card.decisions[0].approver, "Bilal");
        assert_eq!(card.decisions[0].decision, "approve");
        assert_eq!(card.decisions[0].comment, None);
        assert!(card.can_decide && !card.can_withdraw);
        assert_eq!(card.labels["reject"], "Reject");
        assert!(
            LABELS
                .iter()
                .all(|(name, _)| card.labels.contains_key(name))
        );
        assert!(
            card.labels
                .values()
                .all(|text| !text.starts_with("wl_kernel.")),
            "every label has an English text"
        );
    }

    #[test]
    fn a_card_names_an_unknown_user_by_id_and_says_when_the_subject_moved() {
        let mut request = request(ApprovalStatus::Rejected, 1);
        request.requester_id = 9;
        request.subject_moved_to = Some("cancelled".into());
        let card = build_card(
            &request,
            &[],
            &[],
            &people(),
            CardRights::default(),
            Tr(None),
        );
        assert_eq!(card.requester, "User #9");
        assert_eq!(
            card.subject_moved.as_deref(),
            Some("Subject already moved to `cancelled`.")
        );
        assert_eq!(card.status_kind, "error");
        assert_eq!(card.steps[0].state, "closed");
        assert_eq!(card.steps[1].state, "closed");
    }

    #[test]
    fn every_state_and_decision_has_a_text() {
        use ApprovalStatus::*;
        for status in [
            Pending, Approved, Rejected, Returned, Withdrawn, Expired, Failed,
        ] {
            assert!(!status_label(Tr(None), status).starts_with("wl_kernel."));
        }
        for decision in [
            DecisionKind::Approve,
            DecisionKind::Reject,
            DecisionKind::Return,
        ] {
            assert!(!decision_label(Tr(None), decision).starts_with("wl_kernel."));
        }
        for state in ["done", "current", "waiting", "closed"] {
            let key = format!("wl_kernel.approvals.step_state.{state}");
            assert_ne!(texts::en(&key), key);
        }
    }

    #[test]
    fn the_timeline_shows_the_request_each_decision_and_the_end_with_its_reason() {
        let request = request(ApprovalStatus::Rejected, 2);
        let reject = ApprovalDecision {
            id: 2,
            step: 2,
            approver_id: 1,
            decision: DecisionKind::Reject,
            comment: Some("duplicate".into()),
            created_at: at(30),
            ..approved_step_one()
        };
        let items = request_items(
            &request,
            &[approved_step_one(), reject],
            Some("duplicate"),
            &people(),
            Tr(None),
        );
        let titles: Vec<_> = items.iter().map(|i| i["title"].as_str().unwrap()).collect();
        assert_eq!(
            titles,
            [
                "Approval requested: payments.refund",
                "Approved at step 1 by Bilal",
                "Rejected at step 2 by Aisha",
                "Approval rejected: payments.refund",
            ]
        );
        assert_eq!(items[0]["body"], "Requested by Aisha");
        assert!(items[1]["body"].is_null());
        assert_eq!(items[2]["body"], "duplicate");
        assert_eq!(items[3]["body"], "duplicate");
        assert_eq!(items[3]["kind"], "error");
        assert_eq!(items[0]["time"], at(0).unwrap().to_rfc3339());
    }

    #[test]
    fn a_pending_request_has_no_end_item_and_a_moved_subject_is_said() {
        let pending = request_items(
            &request(ApprovalStatus::Pending, 1),
            &[],
            None,
            &people(),
            Tr(None),
        );
        assert_eq!(pending.len(), 1);

        let mut withdrawn = request(ApprovalStatus::Withdrawn, 1);
        withdrawn.subject_moved_to = Some("cancelled".into());
        let items = request_items(
            &withdrawn,
            &[],
            Some("approval withdrawn by requester"),
            &people(),
            Tr(None),
        );
        assert_eq!(items[1]["title"], "Approval withdrawn: payments.refund");
        assert_eq!(
            items[1]["body"],
            "approval withdrawn by requester Subject already moved to `cancelled`."
        );
        assert_eq!(items[1]["kind"], "info");
    }

    #[test]
    fn a_reject_needs_a_comment_that_is_not_blank() {
        let blank = DecisionForm {
            comment: Some("   ".into()),
        };
        assert_eq!(blank.comment(), "");
        assert_eq!(DecisionForm::default().comment(), "");
        let given = DecisionForm {
            comment: Some(" duplicate ".into()),
        };
        assert_eq!(given.comment(), "duplicate");
        assert!(matches!(
            comment_required(Tr(None), &blank),
            Error::Validation(_)
        ));
    }

    #[test]
    fn the_toast_after_a_yes_says_what_happened() {
        assert_eq!(
            approved_toast(Tr(None), Some(ApprovalEnd::Approved)).kind,
            ToastKind::Success
        );
        assert_eq!(
            approved_toast(Tr(None), Some(ApprovalEnd::Failed)).kind,
            ToastKind::Error
        );
        assert_eq!(approved_toast(Tr(None), None).kind, ToastKind::Success);
    }
}
