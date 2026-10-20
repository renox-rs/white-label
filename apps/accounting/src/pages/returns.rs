//! The return pages: the list, a return with its history and signatures, and
//! the actions on it (04 §7.1 *Return*; BL-ACC-16, BL-ACC-17).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/returns` | `accounting.returns` | `returns.view` |
//! | `POST /accounting/returns` | `accounting.return_create` | `returns.prepare` |
//! | `GET /accounting/returns/{id}` | `accounting.return` | `returns.view` |
//! | `POST /accounting/returns/{id}/figures` | `accounting.return_figures` | `returns.prepare` |
//! | `POST /accounting/returns/{id}/submit` | `accounting.return_submit` | `returns.prepare` |
//! | `POST /accounting/returns/{id}/send-back` | `accounting.return_send_back` | `returns.review` |
//! | `POST /accounting/returns/{id}/review` | `accounting.return_review` | `returns.review` |
//! | `POST /accounting/returns/{id}/withdraw` | `accounting.return_withdraw` | `returns.review` |
//! | `POST /accounting/returns/{id}/cancel` | `accounting.return_cancel` | `returns.review` |
//! | `POST /accounting/returns/{id}/file` | `accounting.return_file` | `returns.file` |
//!
//! The client approver's pages (approve, send back, withdraw) are in
//! [`super::portal_returns`]. A button is offered only when the transition
//! can run for the user: no withdrawal and no cancel on a `filed` return,
//! no review by the preparer. The actions answer with a toast and a refresh;
//! a refusal answers 422 with its lines.

use renox::prelude::*;
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::deadlines::deadline_tasks;
use wl_kernel::documents::dubai_text;

use super::{Texts, invalid, parse_id, party_name, require, text, user_name};
use crate::engagement::Engagement;
use crate::permissions;
use crate::reasons;
use crate::returns::{self, NewReturn};
use crate::tax_return::{Return, ReturnSignature, ReturnStatus, ReturnType};
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/returns", index)
        .name("accounting.returns")
        .post("/accounting/returns", create)
        .name("accounting.return_create")
        .get("/accounting/returns/{id}", show)
        .name("accounting.return")
        .post("/accounting/returns/{id}/figures", figures)
        .name("accounting.return_figures")
        .post("/accounting/returns/{id}/submit", submit)
        .name("accounting.return_submit")
        .post("/accounting/returns/{id}/send-back", send_back)
        .name("accounting.return_send_back")
        .post("/accounting/returns/{id}/review", review)
        .name("accounting.return_review")
        .post("/accounting/returns/{id}/withdraw", withdraw)
        .name("accounting.return_withdraw")
        .post("/accounting/returns/{id}/cancel", cancel)
        .name("accounting.return_cancel")
        .post("/accounting/returns/{id}/file", file)
        .name("accounting.return_file")
        .require_auth()
}

/// The badge kind of a return status.
pub(crate) fn status_kind(status: ReturnStatus) -> &'static str {
    match status {
        ReturnStatus::InReview | ReturnStatus::ClientApproval => "info",
        ReturnStatus::Approved | ReturnStatus::Filed => "success",
        ReturnStatus::Preparing | ReturnStatus::Superseded | ReturnStatus::Cancelled => "info",
    }
}

/// The status of a return in the page's language.
pub(crate) fn status_label(lang: &Lang, status: ReturnStatus) -> String {
    text(lang, &format!("returns.status.{}", status.as_str()))
}

/// The type of a return in the page's language.
pub(crate) fn type_label(lang: &Lang, kind: ReturnType) -> String {
    text(lang, &format!("returns.type.{}", kind.as_str()))
}

/// One return of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ReturnRow {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// VAT or corporate tax.
    pub kind: String,
    /// The period's last day.
    pub period: String,
    /// The due date.
    pub due: String,
    /// The version.
    pub version: i64,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// The rows of `returns` as `lang` reads them.
pub(crate) async fn rows(state: &AppState, lang: &Lang, returns: &[Return]) -> Vec<ReturnRow> {
    let mut rows = Vec::with_capacity(returns.len());
    for ret in returns {
        rows.push(ReturnRow {
            id: ret.id,
            number: ret.number(),
            client: party_name(state, lang, ret.client()).await,
            kind: type_label(lang, ret.return_type),
            period: ret.period_end.to_string(),
            due: ret.due_date.to_string(),
            version: ret.version,
            status: status_label(lang, ret.status),
            status_kind: status_kind(ret.status),
        });
    }
    rows
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/returns.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    returns: Vec<ReturnRow>,
    can_create: bool,
    types: Vec<[String; 2]>,
}

/// `GET /accounting/returns`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::RETURNS_VIEW)?;
    let all = Return::query()
        .order_by_desc(Return::ID)
        .get(&state.db)
        .await?;
    let types = ReturnType::ALL
        .iter()
        .map(|t| [t.as_str().to_owned(), type_label(&lang, *t)])
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        returns: rows(&state, &lang, &all).await,
        can_create: user.has_permission(permissions::RETURNS_PREPARE),
        types,
    }))
}

/// The form that opens a return.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The engagement's id.
    #[serde(default)]
    pub engagement: String,
    /// `vat` or `ct`.
    #[serde(default)]
    pub return_type: String,
    /// The period's last day.
    #[serde(default)]
    pub period_end: String,
    /// The due date.
    #[serde(default)]
    pub due_date: String,
}

/// `POST /accounting/returns`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::RETURNS_PREPARE)?;
    let engagement =
        parse_id(&form.engagement).ok_or_else(|| invalid("engagement: 1, 2, 3 ..."))?;
    let return_type = ReturnType::ALL
        .iter()
        .copied()
        .find(|t| t.as_str() == form.return_type.trim())
        .unwrap_or_default();
    let period_end = form
        .period_end
        .trim()
        .parse()
        .map_err(|_| invalid("period_end: YYYY-MM-DD"))?;
    let due_date = form
        .due_date
        .trim()
        .parse()
        .map_err(|_| invalid("due_date: YYYY-MM-DD"))?;
    returns::open(
        &state,
        Some(user.id),
        NewReturn::new(engagement, return_type, period_end, due_date),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "returns.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The return as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct ReturnView {
    /// The primary key.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The client's name.
    pub client: String,
    /// The engagement's number.
    pub engagement: String,
    /// VAT or corporate tax.
    pub kind: String,
    /// The period.
    pub period: String,
    /// The due date.
    pub due: String,
    /// The version.
    pub version: i64,
    /// The preparer.
    pub preparer: String,
    /// The reviewer.
    pub reviewer: String,
    /// The figures.
    pub figures: String,
    /// Whether the workpapers are attached.
    pub workpapers: String,
    /// The linked deadline task and its state.
    pub task: String,
    /// The last comment of a send back.
    pub send_back_comment: String,
    /// Why it was cancelled.
    pub cancel_reason: String,
    /// The day it was filed.
    pub filed_on: String,
    /// The FTA acknowledgement.
    pub acknowledgement: String,
}

/// One row of the history.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// What happened.
    pub action: String,
    /// The reason or comment, if any.
    pub reason: String,
    /// Who ran it, or the system.
    pub by: String,
}

/// One signature of the return.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SignatureRow {
    /// Who signed.
    pub signer: String,
    /// When (Asia/Dubai).
    pub at: String,
    /// The IP address.
    pub ip: String,
    /// The document hash.
    pub hash: String,
    /// "Valid", or "Withdrawn on …: reason".
    pub state: String,
}

/// What a return page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/return.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    ret: ReturnView,
    status: String,
    status_kind: &'static str,
    history: Vec<HistoryRow>,
    signatures: Vec<SignatureRow>,
    can_figures: bool,
    can_submit: bool,
    can_send_back: bool,
    can_review: bool,
    can_withdraw: bool,
    can_cancel: bool,
    can_file: bool,
    cancel_reasons: Vec<[String; 2]>,
    figures_text: String,
}

/// The history of `ret` as `lang` reads it.
async fn history_rows(state: &AppState, lang: &Lang, ret: &Return) -> Result<Vec<HistoryRow>> {
    let mut rows = Vec::new();
    for row in history(&state.db, Return::TABLE, ret.id).await? {
        let by = match row.actor_id {
            Some(id) => user_name(&state.db, id).await,
            None => text(lang, "history.system"),
        };
        let reason = match row.reason.as_deref() {
            Some(code) if reasons::RETURN_CANCEL_REASONS.contains(&code) => {
                reasons::label(lang, code)
            }
            Some(other) => other.to_owned(),
            None => String::new(),
        };
        rows.push(HistoryRow {
            at: dubai_text(row.at),
            action: text(lang, &format!("returns.transition.{}", row.transition)),
            reason,
            by,
        });
    }
    Ok(rows)
}

/// The signatures of `ret` as `lang` reads them.
pub(crate) async fn signature_rows(
    state: &AppState,
    lang: &Lang,
    ret: &Return,
) -> Result<Vec<SignatureRow>> {
    let mut rows = Vec::new();
    for signature in ReturnSignature::of_return(&state.db, ret.id).await? {
        let state_text = match signature.withdrawn_at {
            Some(at) => texts::t_with(
                lang,
                "accounting.returns.signature_withdrawn",
                &[
                    ("at", &dubai_text(at)),
                    (
                        "reason",
                        &signature.withdrawn_reason.clone().unwrap_or_default(),
                    ),
                ],
            ),
            None => text(lang, "returns.signature_valid"),
        };
        rows.push(SignatureRow {
            signer: signature.signer_name.clone(),
            at: dubai_text(signature.signed_at),
            ip: signature.ip_address.clone().unwrap_or_default(),
            hash: signature.document_hash.clone(),
            state: state_text,
        });
    }
    Ok(rows)
}

/// The linked task of `ret` and its state, for people.
pub(crate) async fn task_text(state: &AppState, lang: &Lang, ret: &Return) -> Result<String> {
    let Some(id) = ret.task() else {
        return Ok(String::new());
    };
    Ok(match deadline_tasks(state).find(&state.db, id).await? {
        Some(task) => format!(
            "{} ({})",
            task.title,
            text(lang, &format!("tasks.status.{}", task.status.as_str()))
        ),
        None => String::new(),
    })
}

/// `GET /accounting/returns/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::RETURNS_VIEW)?;
    let found = Return::find_or_404(&state.db, id).await?;
    let engagement = Engagement::find_or_404(&state.db, found.engagement_id).await?;
    let preparer = match found.preparer_user_id {
        Some(id) => user_name(&state.db, id).await,
        None => String::new(),
    };
    let reviewer = match found.reviewer_user_id {
        Some(id) => user_name(&state.db, id).await,
        None => String::new(),
    };
    let view = ReturnView {
        id: found.id,
        number: found.number(),
        client: party_name(&state, &lang, found.client()).await,
        engagement: engagement.number(),
        kind: type_label(&lang, found.return_type),
        period: found.period_end.to_string(),
        due: found.due_date.to_string(),
        version: found.version,
        preparer,
        reviewer,
        figures: found.figures.clone().unwrap_or_default(),
        workpapers: text(
            &lang,
            if found.workpapers_attached {
                "returns.workpapers_attached"
            } else {
                "returns.workpapers_missing"
            },
        ),
        task: task_text(&state, &lang, &found).await?,
        send_back_comment: found.send_back_comment.clone().unwrap_or_default(),
        cancel_reason: found
            .cancel_reason
            .as_deref()
            .map(|code| reasons::label(&lang, code))
            .unwrap_or_default(),
        filed_on: found.filed_on.map(|d| d.to_string()).unwrap_or_default(),
        acknowledgement: found.acknowledgement.clone().unwrap_or_default(),
    };
    let prepare = user.has_permission(permissions::RETURNS_PREPARE);
    let review = user.has_permission(permissions::RETURNS_REVIEW);
    let status = found.status;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        status: status_label(&lang, status),
        status_kind: status_kind(status),
        history: history_rows(&state, &lang, &found).await?,
        signatures: signature_rows(&state, &lang, &found).await?,
        can_figures: prepare && status == ReturnStatus::Preparing,
        can_submit: prepare && status == ReturnStatus::Preparing,
        can_send_back: review
            && matches!(
                status,
                ReturnStatus::InReview | ReturnStatus::ClientApproval
            ),
        can_review: review
            && status == ReturnStatus::InReview
            && found.preparer_user_id != Some(user.id),
        can_withdraw: review && status == ReturnStatus::Approved,
        can_cancel: review && status.is_open(),
        can_file: user.has_permission(permissions::RETURNS_FILE)
            && status == ReturnStatus::Approved,
        cancel_reasons: returns::cancel_reasons()
            .iter()
            .map(|code| [(*code).to_owned(), reasons::label(&lang, code)])
            .collect(),
        figures_text: found.figures.clone().unwrap_or_default(),
        ret: view,
    }))
}

/// A refresh with a toast.
fn done(lang: &Lang, key: &str) -> Response {
    (Toast::success(text(lang, key)), renox::HxRefresh).into_response()
}

/// The form that records the figures.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FiguresForm {
    /// The figures.
    #[serde(default)]
    pub figures: String,
    /// `1` when the workpapers are attached.
    #[serde(default)]
    pub workpapers: String,
}

/// `POST /accounting/returns/{id}/figures`.
async fn figures(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FiguresForm>,
) -> Result<Response> {
    let attached = matches!(form.workpapers.trim(), "1" | "on" | "true");
    returns::record_figures(&state, &user, id, &form.figures, attached).await?;
    Ok(done(&lang, "returns.figures_recorded"))
}

/// `POST /accounting/returns/{id}/submit`.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    returns::submit(&state, &user, id).await?;
    Ok(done(&lang, "returns.submitted"))
}

/// The form of an action with a comment or reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CommentForm {
    /// The comment or reason.
    #[serde(default)]
    pub comment: String,
    /// The client's written instruction (withdrawing as a reviewer).
    #[serde(default)]
    pub instruction: String,
}

/// `POST /accounting/returns/{id}/send-back`.
async fn send_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CommentForm>,
) -> Result<Response> {
    returns::send_back(&state, &user, id, &form.comment).await?;
    Ok(done(&lang, "returns.sent_back"))
}

/// `POST /accounting/returns/{id}/review`.
async fn review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    returns::review(&state, &user, id).await?;
    Ok(done(&lang, "returns.reviewed"))
}

/// `POST /accounting/returns/{id}/withdraw`: a reviewer withdraws the
/// approval on the client's written instruction.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CommentForm>,
) -> Result<Response> {
    returns::withdraw_approval(&state, &user, id, &form.comment, Some(&form.instruction)).await?;
    Ok(done(&lang, "returns.withdrawn"))
}

/// The form of a cancel.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CancelForm {
    /// The reason code.
    #[serde(default)]
    pub reason: String,
}

/// `POST /accounting/returns/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CancelForm>,
) -> Result<Response> {
    returns::cancel(&state, &user, id, &form.reason).await?;
    Ok(done(&lang, "returns.cancelled"))
}

/// The form that marks a return filed.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FileForm {
    /// The FTA acknowledgement.
    #[serde(default)]
    pub acknowledgement: String,
    /// The filer's tax agent registration number.
    #[serde(default)]
    pub registration: String,
    /// `1` when the client filed it itself.
    #[serde(default)]
    pub by_client: String,
}

/// `POST /accounting/returns/{id}/file`.
async fn file(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FileForm>,
) -> Result<Response> {
    let registration = Some(form.registration.trim()).filter(|r| !r.is_empty());
    let by_client = matches!(form.by_client.trim(), "1" | "on" | "true");
    returns::file(
        &state,
        &user,
        id,
        &form.acknowledgement,
        by_client,
        registration,
    )
    .await?;
    Ok(done(&lang, "returns.filed"))
}
