//! The engagement letter: staff send it for acceptance, the client approver
//! signs it in the portal (04 §4.1, `DOC.SIG.01`, `ACC.RUL.11`; BL-ACC-02).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /accounting/engagements/{id}/letter/send` | `accounting.engagement_letter_send` | `engagements.manage` |
//! | `GET /accounting/portal/engagements/{id}/letter` | `accounting.portal_letter` | a portal user acting for the client, or `engagements.view` |
//! | `POST /accounting/portal/engagements/{id}/letter/sign` | `accounting.portal_letter_sign` | a client approver |
//!
//! A user who neither acts for the client nor holds `engagements.view` gets
//! a 404, as does a client user whose grants were closed by a decline.

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use super::{Texts, text};
use crate::deadline_notices::APPROVER_ROLES;
use crate::engagement::{Engagement, EngagementStatus};
use crate::letters::{self, EngagementLetter};
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .post("/accounting/engagements/{id}/letter/send", send)
        .name("accounting.engagement_letter_send")
        .get("/accounting/portal/engagements/{id}/letter", portal_show)
        .name("accounting.portal_letter")
        .post(
            "/accounting/portal/engagements/{id}/letter/sign",
            portal_sign,
        )
        .name("accounting.portal_letter_sign")
        .require_auth()
}

/// `POST /accounting/engagements/{id}/letter/send`: sends the letter (a
/// new version when one is waiting) for acceptance, from today.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    letters::send(&state, &user, id, today()).await?;
    Ok((
        Toast::success(text(&lang, "letters.sent")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What the portal page of the letter needs.
#[derive(Serialize, ViewData)]
#[view(template = "accounting/portal_letter.html")]
pub(crate) struct PortalLetterPage {
    tx: Texts,
    id: i64,
    number: String,
    title: String,
    version: i64,
    sent_on: String,
    state: String,
    can_sign: bool,
}

/// Whether `user` is a client approver of `engagement` today.
async fn is_approver(state: &AppState, user: &AuthUser, engagement: &Engagement) -> Result<bool> {
    Ok(portal_users(state)
        .acting_for(&state.db, user.id, engagement.client(), today())
        .await?
        .is_some_and(|acting| acting.has_any_role(&APPROVER_ROLES)))
}

/// The engagement `id` for `user`, or 404: a client user acting for its
/// client, or staff with `engagements.view`.
async fn visible(state: &AppState, user: &AuthUser, id: i64) -> Result<Engagement> {
    let engagement = Engagement::find_or_404(&state.db, id).await?;
    if user.has_permission(permissions::ENGAGEMENTS_VIEW) {
        return Ok(engagement);
    }
    let acting = portal_users(state)
        .acting_for(&state.db, user.id, engagement.client(), today())
        .await?
        .is_some();
    if acting {
        Ok(engagement)
    } else {
        Err(Error::NotFound)
    }
}

/// `GET /accounting/portal/engagements/{id}/letter`.
async fn portal_show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let engagement = visible(&state, &user, id).await?;
    let letter = EngagementLetter::current(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let approver = is_approver(&state, &user, &engagement).await?;
    let state_text = match letter.signed_on {
        Some(day) => texts::t_with(&lang, "accounting.letters.signed_on", &[("date", &day)]),
        None => text(&lang, "letters.waiting"),
    };
    Ok(View::typed(PortalLetterPage {
        tx: texts::all(&lang),
        id,
        number: engagement.number(),
        title: engagement.title.clone(),
        version: letter.version,
        sent_on: letter.sent_on.to_string(),
        state: state_text,
        can_sign: approver && letter.is_open() && engagement.status == EngagementStatus::Onboarding,
    }))
}

/// `POST /accounting/portal/engagements/{id}/letter/sign`.
async fn portal_sign(
    State(state): State<AppState>,
    user: AuthUser,
    renox::ClientIp(ip): renox::ClientIp,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    visible(&state, &user, id).await?;
    let ip = ip.map(|ip| ip.to_string());
    letters::sign(&state, &user, id, ip.as_deref()).await?;
    Ok((
        Toast::success(text(&lang, "letters.signed")),
        renox::HxRefresh,
    )
        .into_response())
}
