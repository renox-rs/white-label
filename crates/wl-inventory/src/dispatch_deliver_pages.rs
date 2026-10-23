//! The actions on a dispatch that record its delivery (wl-inventory §4
//! *Dispatch* row `confirm_delivery` and *Refused goods coming back*, issue
//! #241):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /dispatches/{id}/confirm` | `wl_inventory.dispatch_confirm` | `dispatch.manage` at the location |
//! | `POST /dispatches/{id}/refused-back` | `wl_inventory.dispatch_refused_back` | `dispatch.manage` at the location |
//!
//! The routes carry no permission guard of their own (the guard counts global
//! roles only); the transition and the handler check it in the scope of the
//! location. A quantity is sent in the base unit of the item, like `8` or
//! `0.5`; an empty field is 0. Refusals are 422 with the sentence of the
//! guard.

use std::collections::BTreeMap;

use renox::prelude::*;

use crate::dispatch::{Dispatch, lines_of, refused};
use crate::dispatch_deliver::{LineQuantity, ReceivedEntry, confirm, receive_refused};
use crate::dispatch_pages::{done, failed};
use crate::pages::parse_quantity;
use crate::permissions;

/// The routes of this file.
pub fn routes() -> Routes {
    Routes::new()
        .post("/dispatches/{id}/confirm", confirm_dispatch)
        .name("wl_inventory.dispatch_confirm")
        .post("/dispatches/{id}/refused-back", refused_back)
        .name("wl_inventory.dispatch_refused_back")
        .require_auth()
}

/// The quantity in field `name` of `form`: empty is 0; `None` for a bad one.
fn quantity(form: &BTreeMap<String, String>, name: &str) -> Option<i64> {
    match form.get(name).map(|value| value.trim()) {
        None | Some("") => Some(0),
        Some(value) => parse_quantity(value),
    }
}

/// `POST /dispatches/{id}/confirm`: confirms a courier's or a collector's
/// delivery with `delivered_{line}`, `refused_{line}` and `reason_{line}`
/// for every line.
async fn confirm_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut entered = Vec::new();
    for line in lines_of(&state.db, id).await? {
        let problem = || {
            refused(crate::texts::said(
                "wl_inventory.dispatch.ui.problem_quantity",
                &[("position", &line.position.to_string())],
            ))
        };
        let delivered = quantity(&form, &format!("delivered_{}", line.id)).ok_or_else(problem)?;
        let refused_milli = quantity(&form, &format!("refused_{}", line.id)).ok_or_else(problem)?;
        let reason = form
            .get(&format!("reason_{}", line.id))
            .map(|reason| reason.trim().to_owned())
            .filter(|reason| !reason.is_empty());
        entered.push(LineQuantity {
            line_id: line.id,
            delivered_milli: delivered,
            refused_milli,
            reason,
        });
    }
    confirm(&state, &user, id, entered).await.map_err(failed)?;
    Ok(done(&lang, "confirmed"))
}

/// `POST /dispatches/{id}/refused-back`: records the refused goods the driver
/// brought back (`received_{line}`) and those missing (`missing_{line}`).
async fn refused_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope()) {
        return Err(Error::Forbidden);
    }
    let mut entries = Vec::new();
    for line in lines_of(&state.db, id).await? {
        let problem = || {
            refused(crate::texts::said(
                "wl_inventory.dispatch.ui.problem_quantity",
                &[("position", &line.position.to_string())],
            ))
        };
        entries.push(ReceivedEntry {
            line_id: line.id,
            received_milli: quantity(&form, &format!("received_{}", line.id))
                .ok_or_else(problem)?,
            missing_milli: quantity(&form, &format!("missing_{}", line.id)).ok_or_else(problem)?,
        });
    }
    receive_refused(&state, Some(user.id), id, &entries, true).await?;
    Ok(done(&lang, "received"))
}
