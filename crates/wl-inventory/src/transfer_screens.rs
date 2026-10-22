//! The transfer screen: the transfers between stores, a form for a new one,
//! and the buttons that send and receive (BL-STK-11 for the warning, §4
//! *Transfer*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /stock/transfers` | `wl_inventory.transfers` | `stock.transfer` or `stock.view` somewhere |
//! | `POST /stock/transfers` | `wl_inventory.transfer_make` | `stock.transfer` at the source |
//!
//! Sending and receiving post to the routes of [`crate::stock_pages`]. A
//! transfer that arrives at a store on hold shows the warning ("Fridge 2 is on
//! temperature hold") in its *Receive* sheet, which then goes on with
//! `held_ok=1`.
//!
//! Decision: the form makes a transfer of one item (one line); a transfer of
//! several lines is made through the service.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;

use crate::hold_service::warnings_for;
use crate::item::Item;
use crate::location::{Location, LocationType};
use crate::lot::{Lot, LotStatus};
use crate::permissions;
use crate::stock_screens::{Texts, amount, item_name, location_name, locations_with, tr, tx, word};
use crate::transfer::{
    NewTransfer, NewTransferLine, Transfer, TransferStatus, create_draft, lines_of,
};

/// The routes of the transfer screen.
pub fn routes() -> Routes {
    Routes::new()
        .get("/stock/transfers", transfers)
        .name("wl_inventory.transfers")
        .post("/stock/transfers", make)
        .name("wl_inventory.transfer_make")
        .require_auth()
}

/// One transfer.
#[derive(Serialize, Debug, Clone)]
pub struct TransferRow {
    id: i64,
    number: String,
    from: String,
    to: String,
    lines: String,
    status: String,
    status_kind: &'static str,
    when: String,
    can_send: bool,
    can_receive: bool,
    warning: String,
}

/// What the transfers page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/transfers.html")]
pub(crate) struct TransfersPage {
    tx: Texts,
    rows: Vec<TransferRow>,
    can_make: bool,
    from: Vec<(String, String)>,
    to: Vec<(String, String)>,
    items: Vec<(String, String)>,
    lots: Vec<(String, String)>,
}

fn status_kind(status: TransferStatus) -> &'static str {
    match status {
        TransferStatus::Draft => "info",
        TransferStatus::InTransit => "warning",
        TransferStatus::Received => "success",
        TransferStatus::Cancelled => "error",
    }
}

/// `GET /stock/transfers`.
async fn transfers(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    if !user.has_permission_anywhere(permissions::STOCK_TRANSFER)
        && !user.has_permission_anywhere(permissions::STOCK_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for transfer in Transfer::query()
        .order_by_desc(Transfer::ID)
        .limit(100)
        .get(&state.db)
        .await?
    {
        let from = Location::find_or_404(&state.db, transfer.from_location_id).await?;
        let to = Location::find_or_404(&state.db, transfer.to_location_id).await?;
        let mut lines = Vec::new();
        let mut item_ids = Vec::new();
        for line in lines_of(&state.db, transfer.id).await? {
            let item = Item::find_or_404(&state.db, line.item_id).await?;
            lines.push(format!(
                "{} · {}",
                item_name(&item, &lang),
                amount(line.quantity_milli, &item.base_unit)
            ));
            item_ids.push(line.item_id);
        }
        let in_transit = transfer.status == TransferStatus::InTransit;
        let can_receive =
            in_transit && user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.to_scope());
        let warning = if can_receive {
            warnings_for(&state.db, transfer.to_location_id, &item_ids)
                .await?
                .join(" ")
        } else {
            String::new()
        };
        let when = transfer
            .received_at
            .or(transfer.sent_at)
            .or(transfer.created_at)
            .map(dubai_text)
            .unwrap_or_default();
        rows.push(TransferRow {
            id: transfer.id,
            number: transfer.number.clone().unwrap_or_default(),
            from: location_name(&from),
            to: location_name(&to),
            lines: lines.join(", "),
            status: word(&lang, "transfer", transfer.status.as_str()),
            status_kind: status_kind(transfer.status),
            when,
            can_send: transfer.status == TransferStatus::Draft
                && user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.from_scope()),
            can_receive,
            warning,
        });
    }
    let from: Vec<(String, String)> = locations_with(&state, &user, permissions::STOCK_TRANSFER)
        .await?
        .iter()
        .map(|location| (location.id.to_string(), location_name(location)))
        .collect();
    let to = Location::query()
        .where_(Location::LOCATION_TYPE.ne(LocationType::InTransit))
        .order_by(Location::NAME_EN)
        .get(&state.db)
        .await?
        .iter()
        .map(|location| (location.id.to_string(), location_name(location)))
        .collect();
    let mut items = Vec::new();
    for item in Item::query()
        .order_by(Item::NAME_EN)
        .limit(500)
        .get(&state.db)
        .await?
    {
        items.push((item.id.to_string(), item_name(&item, &lang)));
    }
    let mut lots = vec![(String::new(), tr(&lang, "no_lot"))];
    for lot in Lot::query()
        .where_(Lot::STATUS.eq(LotStatus::Released))
        .order_by(Lot::ITEM_ID)
        .get(&state.db)
        .await?
    {
        let item = Item::find_or_404(&state.db, lot.item_id).await?;
        lots.push((
            lot.id.to_string(),
            format!("{} · {}", item_name(&item, &lang), lot.number),
        ));
    }
    Ok(View::typed(TransfersPage {
        tx: tx(&lang),
        rows,
        can_make: !from.is_empty(),
        from,
        to,
        items,
        lots,
    }))
}

/// What a user enters to prepare a transfer.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct MakeForm {
    /// The id of the store the stock leaves.
    #[validate(max = 20)]
    pub from: Option<String>,
    /// The id of the store it goes to.
    #[validate(max = 20)]
    pub to: Option<String>,
    /// The id of the item.
    #[validate(max = 20)]
    pub item: Option<String>,
    /// The id of the lot, for a lot-tracked item.
    #[validate(max = 20)]
    pub lot: Option<String>,
    /// The quantity in the item's base unit, like `5` or `0.5`.
    #[validate(max = 32)]
    pub quantity: Option<String>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn id_of(field: &Option<String>) -> Option<i64> {
    field
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok())
}

/// `POST /stock/transfers`: prepares a draft transfer of one item. The user
/// needs `stock.transfer` at the source.
async fn make(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<MakeForm>,
) -> Result<Response> {
    let from_id = id_of(&form.from).ok_or_else(|| refused(tr(&lang, "problem.from")))?;
    let to_id = id_of(&form.to).ok_or_else(|| refused(tr(&lang, "problem.to")))?;
    let item_id = id_of(&form.item).ok_or_else(|| refused(tr(&lang, "problem.item")))?;
    let quantity_milli = crate::pages::parse_quantity(form.quantity.as_deref().unwrap_or_default())
        .filter(|quantity| *quantity > 0)
        .ok_or_else(|| refused(tr(&lang, "problem.quantity")))?;
    let from = Location::find_or_404(&state.db, from_id).await?;
    if !user.has_permission_in(permissions::STOCK_TRANSFER, &from.scope()) {
        return Err(Error::Forbidden);
    }
    let mut line = NewTransferLine::of(item_id, quantity_milli);
    if let Some(lot_id) = id_of(&form.lot) {
        let lot = Lot::find_or_404(&state.db, lot_id).await?;
        if lot.item_id != item_id {
            return Err(refused(tr(&lang, "problem.lot")));
        }
        line = line.lot(lot_id);
    }
    create_draft(
        &state.db,
        Some(user.id),
        NewTransfer {
            from_location_id: from_id,
            to_location_id: to_id,
            lines: vec![line],
        },
    )
    .await?;
    Ok((Toast::success(tr(&lang, "transfer_made")), renox::HxRefresh).into_response())
}
