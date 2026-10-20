//! The statutory deadline notice in the portal and its sending by the
//! partner or the MLRO (`ACC.RUL.09`, BL-ACC-04).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /accounting/portal/deadline-notices/{id}` | `accounting.deadline_notice` | a portal user acting for the client, or `engagements.view` |
//! | `POST /accounting/portal/deadline-notices/{id}/acknowledge` | `accounting.deadline_notice_acknowledge` | a client approver (`owner` or `approver` grant) |
//! | `POST /accounting/deadline-notices/{id}/send` | `accounting.deadline_notice_send` | the partner's `engagements.end`, or the MLRO's `parties.block_reason.view` for a restricted end |
//!
//! Decision: the portal has no acknowledgement of its own yet, so the page
//! is the app's, reached from the link in the notice; it asks the kernel's
//! `PortalUsers` (wl-portal's access grants) who acts for the client. A
//! notice not sent, or a user who neither acts for the client nor holds
//! `engagements.view`, gets a 404.

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_text;

use super::{Texts, text};
use crate::deadline_notices::{self, APPROVER_ROLES, DeadlineNotice};
use crate::engagement::Engagement;
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/accounting/portal/deadline-notices/{id}", show)
        .name("accounting.deadline_notice")
        .post(
            "/accounting/portal/deadline-notices/{id}/acknowledge",
            acknowledge,
        )
        .name("accounting.deadline_notice_acknowledge")
        .post("/accounting/deadline-notices/{id}/send", send)
        .name("accounting.deadline_notice_send")
        .require_auth()
}

/// One deadline of the notice.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LineRow {
    /// "2026-11-28 — VAT return, period ending 2026-10-31".
    pub line: String,
}

/// What the notice page needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/deadline_notice.html")]
pub(crate) struct NoticePage {
    tx: Texts,
    id: i64,
    number: String,
    wording: String,
    lines: Vec<LineRow>,
    empty: String,
    acknowledged: String,
    can_acknowledge: bool,
}

/// `GET /accounting/portal/deadline-notices/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let notice = DeadlineNotice::find_or_404(&state.db, id).await?;
    let staff = user.has_permission(permissions::ENGAGEMENTS_VIEW);
    let acting = deadline_notices::acting_for(&state, user.id, &notice).await?;
    if notice.sent_at.is_none() && !staff {
        return Err(Error::NotFound);
    }
    if acting.is_none() && !staff {
        return Err(Error::NotFound);
    }
    let engagement = Engagement::find_or_404(&state.db, notice.engagement_id).await?;
    let number = engagement.number();
    let acknowledged = match notice.acknowledged_at {
        Some(at) => texts::t_with(
            &lang,
            "accounting.deadline_notices.acknowledged",
            &[("at", &dubai_text(at))],
        ),
        None => String::new(),
    };
    let can_acknowledge = notice.is_open()
        && acting
            .as_ref()
            .is_some_and(|user| user.has_any_role(&APPROVER_ROLES));
    Ok(View::typed(NoticePage {
        tx: texts::all(&lang),
        id: notice.id,
        wording: deadline_notices::wording(notice.cause, &number),
        number,
        lines: notice
            .line_list()
            .into_iter()
            .map(|line| LineRow { line })
            .collect(),
        empty: texts::t_with(
            &lang,
            "accounting.deadline_notices.none",
            &[("days", &deadline_notices::WINDOW_DAYS)],
        ),
        acknowledged,
        can_acknowledge,
    }))
}

/// `POST /accounting/portal/deadline-notices/{id}/acknowledge`.
async fn acknowledge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    deadline_notices::acknowledge(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "deadline_notices.acknowledged_toast")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /accounting/deadline-notices/{id}/send`.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    deadline_notices::send_prepared(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "deadline_notices.sent")),
        renox::HxRefresh,
    )
        .into_response())
}
