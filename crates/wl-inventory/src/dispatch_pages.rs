//! The actions on a dispatch (wl-inventory §4 *Dispatch*):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /dispatches/{id}/pick` | `wl_inventory.dispatch_pick` | `dispatch.manage` at the location |
//! | `POST /dispatches/{id}/dispatch` | `wl_inventory.dispatch_send` | `dispatch.manage` at the location |
//! | `POST /dispatches/{id}/cancel` | `wl_inventory.dispatch_cancel` | `dispatch.manage` at the location |
//! | `POST /dispatches/{id}/repick` | `wl_inventory.dispatch_repick` | `dispatch.manage` at the location |
//! | `GET /dispatches/{id}/delivery-note` | `wl_inventory.dispatch_note` | `dispatch.manage` or `stock.view` at the location |
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a dispatcher may hold the
//! permission at one site store. The handlers and the transitions check it in
//! the scope of the location. A refusal is a 422 with the sentence of the
//! guard (it shows in the sheet that sent the form); a quantity is sent in the
//! base unit of the item, like `8` or `0.5`. The pages are in
//! [`crate::dispatch_screens`].

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::dispatch::{Dispatch, refused};
use crate::dispatch_pick::{cancel, pick, repick};
use crate::dispatch_send::{HandoverChoice, dispatch};
use crate::permissions;
use crate::texts::t;

/// The routes of this file.
pub fn routes() -> Routes {
    Routes::new()
        .post("/dispatches/{id}/pick", pick_dispatch)
        .name("wl_inventory.dispatch_pick")
        .post("/dispatches/{id}/dispatch", send_dispatch)
        .name("wl_inventory.dispatch_send")
        .post("/dispatches/{id}/cancel", cancel_dispatch)
        .name("wl_inventory.dispatch_cancel")
        .post("/dispatches/{id}/repick", repick_dispatch)
        .name("wl_inventory.dispatch_repick")
        .get("/dispatches/{id}/delivery-note", delivery_note)
        .name("wl_inventory.dispatch_note")
        .require_auth()
}

/// The text `key` of the dispatch screens.
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    t(lang, &format!("wl_inventory.dispatch.ui.{key}"))
}

/// The toast and the refresh that answer a done action.
pub(crate) fn done(lang: &Lang, key: &str) -> Response {
    (Toast::success(text(lang, key)), renox::HxRefresh).into_response()
}

/// A 422 for a guard, the error otherwise.
pub(crate) fn failed(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// `POST /dispatches/{id}/pick`: picks a draft dispatch and reserves its
/// stock.
async fn pick_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    pick(&state, &user, id).await.map_err(failed)?;
    Ok(done(&lang, "picked"))
}

/// What the dispatcher enters to dispatch.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct SendForm {
    /// `own_delivery`, `courier` or `collection`.
    #[validate(max = 32)]
    pub handover: Option<String>,
    /// The courier's tracking number.
    #[validate(max = 200)]
    pub tracking_number: Option<String>,
    /// The collector's signature reference.
    #[validate(max = 200)]
    pub signature: Option<String>,
}

/// `POST /dispatches/{id}/dispatch`: sends a picked dispatch. A lot on hold
/// or a failed credit re-check is a 422 and the dispatch stays picked.
async fn send_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<SendForm>,
) -> Result<Response> {
    let kind = form.handover.unwrap_or_default();
    let choice = HandoverChoice {
        kind,
        tracking_number: form.tracking_number,
        signature: form.signature,
    };
    dispatch(&state, &user, id, &choice).await.map_err(failed)?;
    Ok(done(&lang, "dispatched"))
}

/// What the dispatcher enters to cancel.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CancelForm {
    /// Why.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /dispatches/{id}/cancel`: cancels a draft or picked dispatch.
async fn cancel_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    cancel(&state, &user, id, form.reason.as_deref())
        .await
        .map_err(failed)?;
    Ok(done(&lang, "cancelled"))
}

/// What the dispatcher enters to re-pick a line.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct RepickForm {
    /// The line to pick again.
    #[validate(max = 20)]
    pub line: Option<String>,
    /// The lot to pick it from; empty takes the first to expire.
    #[validate(max = 20)]
    pub lot: Option<String>,
}

/// `POST /dispatches/{id}/repick`: picks a line again from another lot.
async fn repick_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<RepickForm>,
) -> Result<Response> {
    let line = form
        .line
        .as_deref()
        .and_then(|line| line.trim().parse().ok())
        .ok_or(Error::NotFound)?;
    let lot = form
        .lot
        .as_deref()
        .map(str::trim)
        .filter(|lot| !lot.is_empty())
        .map(|lot| lot.parse::<i64>().map_err(|_| Error::NotFound))
        .transpose()?;
    repick(&state, &user, id, line, lot).await?;
    Ok(done(&lang, "repicked"))
}

/// `GET /dispatches/{id}/delivery-note`: the delivery note, a PDF.
async fn delivery_note(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope())
        && !user.has_permission_in(permissions::STOCK_VIEW, &dispatch.scope())
    {
        return Err(Error::Forbidden);
    }
    let note = dispatch.delivery_note.ok_or(Error::NotFound)?;
    Ok(([("content-type", "application/pdf")], note).into_response())
}
