//! The approvals inbox (`GET /approvals`, BL-MPW-29, `docs/UX.md` §2.1): the
//! decisions waiting for the signed-in person, each in words, and the
//! requests the person made themselves.
//!
//! - **Waiting for you** is [`approvals::awaiting_items`]' source: the pending
//!   requests whose current step the person may decide. A row names the action
//!   in words ([`approvals::awaiting::action_label`]; an action nobody gave
//!   words shows *Approval needed*), the record by its name and not its id,
//!   who asked, on which day and why, links to the record, and offers
//!   *Approve*, *Reject* and *Return for changes*: the sheets post to the
//!   kernel's decide routes (`wl_kernel.approvals.approve|reject|return`), which
//!   keep the rules: the requester never decides (they are not in this list),
//!   *Reject* and *Return* need a note.
//! - **My requests** are the person's own pending requests with who they wait
//!   for (*Waiting for the MD*, or *Waiting for another user* when the step is
//!   a permission), and the ones returned to them with the approver's note.
//! - The reason is read from the request's payload (`reason`, `note`, `text`,
//!   `comment`), the only place the kernel keeps it; a request without one
//!   shows no reason line.
//!
//! Decision: the subject's name comes from the registered subject label, else
//! from [`subject_text`] for the Manpower and HR records, else nothing: the
//! page never shows `hr.employee #2`. A request whose subject moved on shows
//! no *Dismiss* (not built): the kernel ends such a request itself.

use renox::nav::NavLink;
use renox::prelude::*;
use renox::serde_json::Value;
use serde::Serialize;
use wl_kernel::approvals::{
    self, ApprovalCard, ApprovalRequest, ApprovalStatus, awaiting::action_label,
};
use wl_kernel::{BusinessCalendar, Subject};

/// The inbox module.
pub struct ApprovalsInbox;

impl Module for ApprovalsInbox {
    fn name(&self) -> &'static str {
        "approvals_inbox"
    }

    fn register(&self, app: &mut Registry) {
        approvals::register_subject_link("hr.employee", |id| format!("/hr/employees/{id}"));
        app.nav(
            "app",
            NavLink::route("approvals.inbox", "nav.approvals")
                .icon("circle-check")
                .order(1)
                .when(|user| !wl_kernel::portal::is_portal_only(user)),
        );
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .get("/approvals", index)
            .name("approvals.inbox")
            .require_auth()
            .require_verified()
    }
}

/// One approval, as the page shows it.
#[derive(Serialize, Debug, Clone)]
struct Row {
    id: i64,
    /// What is asked, in words.
    action: String,
    /// The record, by its name; empty when it has none.
    subject: String,
    /// The record's page, when its module has one.
    url: String,
    requester: String,
    /// `Requested by Amal Youssef on 01 Nov 2026`.
    requested: String,
    reason: String,
    status: String,
    status_kind: &'static str,
    /// Why the person cannot decide: who it waits for, or the returned note.
    state: String,
    can_decide: bool,
    can_withdraw: bool,
    labels: std::collections::BTreeMap<&'static str, String>,
}

/// The first non-empty text of the payload under a name a reason goes by.
fn reason_of(payload: &Value) -> String {
    ["reason", "note", "text", "comment", "waiver_reason"]
        .iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// A role key as the words of whoever holds it.
fn role_words(role: &str) -> String {
    match role {
        "md" => "the MD".to_owned(),
        "hr" => "HR".to_owned(),
        "admin" | "company_admin" => "the admin".to_owned(),
        "account_manager" => "the account manager".to_owned(),
        other => format!("the {}", other.replace('_', " ")),
    }
}

/// Who a request waits for, in words.
fn waiting_for(lang: &Lang, request: &ApprovalRequest) -> String {
    match request.step().and_then(|step| step.roles.first()) {
        Some(role) => lang.t("approvals.waiting_for", &[("who", &role_words(role))]),
        None => lang.t("approvals.waiting_other", &[]),
    }
}

/// The name of the record a request is about, or empty.
async fn subject_text(state: &AppState, lang: &Lang, subject: &Subject) -> String {
    if let Ok(Some(label)) = wl_kernel::documents::subject_label(&state.db, subject).await {
        return label;
    }
    if let Some(name) = manpower::approvals_text::subject_name(state, subject, &lang.locale).await {
        return name;
    }
    if let Some(name) = hr_subject(state, lang, subject).await {
        return name;
    }
    invoicing_subject(state, lang, subject)
        .await
        .unwrap_or_default()
}

/// The name of an invoice or a rate card version.
async fn invoicing_subject(state: &AppState, lang: &Lang, subject: &Subject) -> Option<String> {
    match subject.subject_type.as_str() {
        "invoicing.rate_card_version" => {
            let version =
                wl_invoicing::rate_card::RateCardVersion::find(&state.db, subject.subject_id)
                    .await
                    .ok()??;
            let card = wl_invoicing::rate_card::RateCard::find(&state.db, version.rate_card_id)
                .await
                .ok()??;
            let client = wl_kernel::parties::directory(state)
                .display_name(&state.db, wl_kernel::PartyId(card.party_id), &lang.locale)
                .await
                .ok()??;
            Some(lang.t(
                "approvals.rate_card_of",
                &[
                    ("client", &client),
                    ("date", &version.effective_from.format("%d %b %Y")),
                ],
            ))
        }
        "invoicing.invoice" => {
            wl_invoicing::Invoice::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .number
        }
        _ => None,
    }
}

/// The employee's name, for the HR records.
async fn hr_subject(state: &AppState, lang: &Lang, subject: &Subject) -> Option<String> {
    let employee_id = match subject.subject_type.as_str() {
        "hr.employee" => subject.subject_id,
        "hr.pay_account" => {
            wl_hr::PayAccount::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .employee_id
        }
        "hr.settlement" => {
            wl_hr::FinalSettlement::find(&state.db, subject.subject_id)
                .await
                .ok()??
                .employee_id
        }
        _ => return None,
    };
    let employee = wl_hr::Employee::find(&state.db, employee_id).await.ok()??;
    wl_kernel::parties::directory(state)
        .display_name(
            &state.db,
            wl_kernel::PartyId(employee.party_id),
            &lang.locale,
        )
        .await
        .ok()?
}

async fn row(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    request: &ApprovalRequest,
) -> Result<Row> {
    let card: ApprovalCard = approvals::card(&state.db, request, user, lang).await?;
    let day = request.created_at.map_or_else(String::new, |at| {
        at.with_timezone(&BusinessCalendar::offset())
            .format("%d %b %Y")
            .to_string()
    });
    let subject = request.subject();
    // Built before the awaits below: a `&dyn Display` list is not `Send`.
    let requested = lang.t(
        "approvals.requested",
        &[("name", &card.requester), ("date", &day)],
    );
    let reason = match reason_of(&request.payload.0) {
        reason if !reason.is_empty() => reason,
        _ => manpower::approvals_text::request_reason(state, request, &lang.locale)
            .await
            .unwrap_or_default(),
    };
    Ok(Row {
        id: request.id,
        action: action_label(lang, &request.action),
        subject: subject_text(state, lang, &subject).await,
        url: approvals::subject_url(&request.subject_type, request.subject_id).unwrap_or_default(),
        requester: card.requester.clone(),
        requested,
        reason,
        status: card.status_label.clone(),
        status_kind: card.status_kind,
        state: String::new(),
        can_decide: card.can_decide,
        can_withdraw: card.can_withdraw,
        labels: card.labels,
    })
}

/// What the inbox shows.
async fn build(state: &AppState, lang: &Lang, user: &AuthUser) -> Result<(Vec<Row>, Vec<Row>)> {
    let mut waiting = Vec::new();
    for request in approvals::awaiting::awaiting(&state.db, user).await? {
        waiting.push(row(state, lang, user, &request).await?);
    }
    let mut mine = Vec::new();
    for request in ApprovalRequest::query()
        .where_(ApprovalRequest::REQUESTER_ID.eq(user.id))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .order_by_desc(ApprovalRequest::ID)
        .limit(50)
        .get(&state.db)
        .await?
    {
        let mut line = row(state, lang, user, &request).await?;
        line.state = waiting_for(lang, &request);
        mine.push(line);
    }
    for item in approvals::needs_action_items(&state.db, user, lang).await? {
        let Some(request) = ApprovalRequest::find(&state.db, item.request_id).await? else {
            continue;
        };
        let mut line = row(state, lang, user, &request).await?;
        line.state = match &item.comment {
            Some(note) => lang.t("approvals.returned_with", &[("note", note)]),
            None => lang.t("approvals.returned", &[]),
        };
        mine.push(line);
    }
    Ok((waiting, mine))
}

/// `GET /approvals`: the page for a browser, the rows as JSON otherwise.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: renox::axum::http::HeaderMap,
) -> Result<Response> {
    let (waiting, mine) = build(&state, &lang, &user).await?;
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(
            Json(renox::serde_json::json!({ "waiting": waiting, "mine": mine })).into_response(),
        );
    }
    Ok(view("approvals/index.html", context! { waiting, mine }).into_response())
}
