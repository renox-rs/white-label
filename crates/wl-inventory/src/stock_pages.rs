//! The actions on stock (wl-inventory §4 *Stock hold*, *Transfer*):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /stock-holds` | `wl_inventory.hold_place` | `stock.hold` (at the location, for a hold on a location) |
//! | `POST /stock-issues` | `wl_inventory.stock_issue` | `stock.issue` at the location |
//! | `POST /transfers/{id}/send` | `wl_inventory.transfer_send` | `stock.transfer` at the source |
//! | `POST /transfers/{id}/receive` | `wl_inventory.transfer_receive` | `stock.transfer` at the destination |
//!
//! Receiving stock into a location that is on hold warns first ("Fridge 2 is
//! on temperature hold"): the answer is a 422 with the warning until the
//! form says `held_ok=1`; then the transfer is received and the quantity
//! joins the hold ([`crate::arrival`]).
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a storekeeper may hold the
//! permission at one site store. The handlers and the transitions check it in
//! the scope of the location.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::hold_service::{NewHold, NewScope, place_as, warnings_for};
use crate::permissions;
use crate::texts;
use crate::transfer::{Transfer, TransferStatus, lines_of};

/// The routes of this file.
pub fn routes() -> Routes {
    Routes::new()
        .post("/stock-holds", place_hold)
        .name("wl_inventory.hold_place")
        .post("/stock-issues", issue_stock)
        .name("wl_inventory.stock_issue")
        .post("/transfers/{id}/send", send_transfer)
        .name("wl_inventory.transfer_send")
        .post("/transfers/{id}/receive", receive_transfer)
        .name("wl_inventory.transfer_receive")
        .require_auth()
}

/// The package text `key`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_inventory.{key}"))
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The ids in `text` (`3, 4`); `None` when one is not a number.
fn ids(text: &Option<String>) -> Option<Vec<i64>> {
    let mut found = Vec::new();
    for part in text
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        found.push(part.parse().ok()?);
    }
    Some(found)
}

/// What a user enters to place a hold.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct HoldForm {
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    #[validate(max = 32)]
    pub reason: Option<String>,
    /// The record that caused it, like `ticketing.ticket:5`.
    #[validate(max = 100)]
    pub reference: Option<String>,
    /// The ids of the lots to hold, like `4, 5`: a hold on whole lots.
    #[validate(max = 200)]
    pub lots: Option<String>,
    /// The id of the location to hold: a hold on a location.
    #[validate(max = 20)]
    pub location: Option<String>,
    /// The ids of the items held at the location; all of them when empty.
    #[validate(max = 200)]
    pub items: Option<String>,
}

/// `POST /stock-holds`: places a hold on whole lots or on a location.
async fn place_hold(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<HoldForm>,
) -> Result<Response> {
    let bad = || refused(text(&lang, "hold.bad_form"));
    let reference = form
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(bad)?
        .parse::<Subject>()
        .map_err(|_| bad())?;
    let lots = ids(&form.lots).ok_or_else(bad)?;
    let items = ids(&form.items).ok_or_else(bad)?;
    let location = form
        .location
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<i64>().map_err(|_| bad()))
        .transpose()?;
    let scope = match (lots.is_empty(), location) {
        (false, None) => NewScope::Lots(lots),
        (true, Some(location_id)) => NewScope::Location {
            location_id,
            item_ids: items,
        },
        _ => return Err(bad()),
    };
    place_as(
        &state,
        &user,
        NewHold {
            reason: form.reason.clone().unwrap_or_default(),
            reference,
            scope,
            by: None,
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "hold.placed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What a user enters to issue stock.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct IssueForm {
    /// The id of the item.
    #[validate(max = 20)]
    pub item: Option<String>,
    /// The id of the location.
    #[validate(max = 20)]
    pub location: Option<String>,
    /// The quantity in the item's base unit, like `5` or `0.5`.
    #[validate(max = 32)]
    pub quantity: Option<String>,
    /// The id of the lot, when it matters which one.
    #[validate(max = 20)]
    pub lot: Option<String>,
    /// The cost object, like `construction.cost_code:3`.
    #[validate(max = 100)]
    pub cost_object: Option<String>,
    /// Why.
    #[validate(max = 200)]
    pub reason: Option<String>,
}

/// The id in `field`, `None` when empty.
fn id_of(field: &Option<String>) -> Result<Option<i64>> {
    match field.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| refused("an id must be a number")),
    }
}

/// `POST /stock-issues`: issues stock to a cost object. Refused when the
/// quantity is short or part of it is on hold.
async fn issue_stock(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<IssueForm>,
) -> Result<Response> {
    let item_id = id_of(&form.item)?.ok_or_else(|| refused("item: required"))?;
    let location_id = id_of(&form.location)?.ok_or_else(|| refused("location: required"))?;
    let quantity_milli = crate::pages::parse_quantity(form.quantity.as_deref().unwrap_or_default())
        .ok_or_else(|| refused("quantity: not a quantity"))?;
    let cost_object = match form
        .cost_object
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => None,
        Some(value) => Some(
            value
                .parse::<Subject>()
                .map_err(|_| refused("cost_object: like construction.cost_code:3"))?,
        ),
    };
    crate::stock_service::issue(
        &state,
        &user,
        crate::stock_service::IssueRequest {
            item_id,
            location_id,
            quantity_milli,
            lot_id: id_of(&form.lot)?,
            cost_object,
            reason: form.reason.clone().filter(|r| !r.trim().is_empty()),
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "issue.done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /transfers/{id}/send`: sends a draft transfer. Refused with the
/// first problem: stock not available, a lot not released, held quantity.
async fn send_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    match crate::transfer::send(&state, &user, id).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "transfer.sent")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}

/// What the receiver enters.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReceiveForm {
    /// `1` to go on after the warning that the destination is on hold.
    #[validate(max = 8)]
    pub held_ok: Option<String>,
}

/// `POST /transfers/{id}/receive`: receives an in-transit transfer. When the
/// destination is on hold the answer is the warning until `held_ok=1`.
async fn receive_transfer(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReceiveForm>,
) -> Result<Response> {
    let transfer = Transfer::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.to_scope()) {
        return Err(Error::Forbidden);
    }
    if transfer.status == TransferStatus::InTransit && form.held_ok.as_deref() != Some("1") {
        let items: Vec<i64> = lines_of(&state.db, id)
            .await?
            .iter()
            .map(|line| line.item_id)
            .collect();
        let warnings = warnings_for(&state.db, transfer.to_location_id, &items).await?;
        if !warnings.is_empty() {
            return Err(refused(format!(
                "{} {}",
                warnings.join(" "),
                text(&lang, "hold.go_on")
            )));
        }
    }
    match crate::transfer::receive(&state, &user, id).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "transfer.received")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}
