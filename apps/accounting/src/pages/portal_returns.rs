//! A return in the portal: the client approver approves it, sends it back or
//! withdraws the approval (04 §7.1 *Return*, `ACC.ACL.11`; BL-ACC-16,
//! BL-ACC-17).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/portal/returns/{id}` | `accounting.portal_return` | a portal user acting for the client, or `returns.view` |
//! | `POST /accounting/portal/returns/{id}/approve` | `accounting.portal_return_approve` | a client approver |
//! | `POST /accounting/portal/returns/{id}/send-back` | `accounting.portal_return_send_back` | a client approver |
//! | `POST /accounting/portal/returns/{id}/withdraw` | `accounting.portal_return_withdraw` | a client approver |
//!
//! A user who neither acts for the client nor holds `returns.view` gets a
//! 404, as does a client user once the portal has closed (90 days after the
//! engagement ended, `POR.ACL.01`). While the engagement has ended and the
//! portal is still open, the page is read-only: no action is offered and the
//! services refuse the transitions. Withdrawing is offered on an `approved`
//! return only: a `filed` return offers none.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use super::returns::{
    SignatureRow, signature_rows, status_kind, status_label, task_text, type_label,
};
use super::{Texts, text};
use crate::permissions;
use crate::portal;
use crate::returns;
use crate::tax_return::{Return, ReturnStatus};
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/portal/returns/{id}", show)
        .name("accounting.portal_return")
        .post("/accounting/portal/returns/{id}/approve", approve)
        .name("accounting.portal_return_approve")
        .post("/accounting/portal/returns/{id}/send-back", send_back)
        .name("accounting.portal_return_send_back")
        .post("/accounting/portal/returns/{id}/withdraw", withdraw)
        .name("accounting.portal_return_withdraw")
        .require_auth()
}

/// What the portal page of a return needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/portal_return.html")]
pub(crate) struct PortalReturnPage {
    tx: Texts,
    id: i64,
    number: String,
    kind: String,
    period: String,
    due: String,
    version: i64,
    status: String,
    status_kind: &'static str,
    summary: String,
    client_files: String,
    filed: String,
    signatures: Vec<SignatureRow>,
    task: String,
    can_approve: bool,
    can_send_back: bool,
    can_withdraw: bool,
    read_only: bool,
}

/// The return `id` for `user`, or 404: a client user acting for its client
/// while the portal is open, or staff with `returns.view`.
async fn visible(state: &AppState, user: &AuthUser, id: i64) -> Result<Return> {
    let ret = Return::find_or_404(&state.db, id).await?;
    let staff = user.has_permission(permissions::RETURNS_VIEW);
    let acting = portal_users(state)
        .acting_for(&state.db, user.id, ret.client(), today())
        .await?
        .is_some();
    if staff {
        return Ok(ret);
    }
    if !acting || !portal::is_open(state, ret.client(), today()).await? {
        return Err(Error::NotFound);
    }
    Ok(ret)
}

/// `GET /accounting/portal/returns/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let ret = visible(&state, &user, id).await?;
    let approver = returns::is_client_approver(&state, &user, &ret).await?;
    let client_files = if portal::client_files(&ret) {
        texts::t_with(
            &lang,
            "accounting.portal.you_file",
            &[("due", &ret.due_date)],
        )
    } else {
        String::new()
    };
    let filed = match ret.filed_on {
        Some(day) => texts::t_with(&lang, "accounting.returns.filed_on_date", &[("date", &day)]),
        None => String::new(),
    };
    let read_only = portal::is_read_only(&state, ret.client()).await?;
    Ok(View::typed(PortalReturnPage {
        tx: texts::all(&lang),
        id: ret.id,
        number: ret.number(),
        kind: type_label(&lang, ret.return_type),
        period: ret.period_end.to_string(),
        due: ret.due_date.to_string(),
        version: ret.version,
        status: status_label(&lang, ret.status),
        status_kind: status_kind(ret.status),
        summary: returns::summary(&ret),
        client_files,
        filed,
        signatures: signature_rows(&state, &lang, &ret).await?,
        task: task_text(&state, &lang, &ret).await?,
        can_approve: approver && ret.status == ReturnStatus::ClientApproval,
        can_send_back: approver && ret.status == ReturnStatus::ClientApproval,
        can_withdraw: approver && ret.status == ReturnStatus::Approved,
        read_only,
    }))
}

/// `POST /accounting/portal/returns/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    renox::ClientIp(ip): renox::ClientIp,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    visible(&state, &user, id).await?;
    let ip = ip.map(|ip| ip.to_string());
    returns::approve(&state, &user, id, ip.as_deref()).await?;
    Ok((
        Toast::success(text(&lang, "returns.approved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of an action with a comment.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CommentForm {
    /// The comment or reason.
    #[serde(default)]
    pub comment: String,
}

/// `POST /accounting/portal/returns/{id}/send-back`.
async fn send_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CommentForm>,
) -> Result<Response> {
    visible(&state, &user, id).await?;
    returns::send_back(&state, &user, id, &form.comment).await?;
    Ok((
        Toast::success(text(&lang, "returns.sent_back")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /accounting/portal/returns/{id}/withdraw`: the client approver
/// withdraws the approval with a reason; the signature is kept.
async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CommentForm>,
) -> Result<Response> {
    visible(&state, &user, id).await?;
    returns::withdraw_approval(&state, &user, id, &form.comment, None).await?;
    Ok((
        Toast::success(text(&lang, "returns.withdrawn")),
        renox::HxRefresh,
    )
        .into_response())
}
