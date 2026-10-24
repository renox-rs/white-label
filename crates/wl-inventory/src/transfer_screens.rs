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

use renox::grid::{Column, Grid, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use crate::hold_service::warnings_for;
use crate::item::Item;
use crate::location::{Location, LocationType};
use crate::lot::{Lot, LotStatus};
use crate::permissions;
use crate::stock_screens::{
    Texts, amount, exported, item_name, location_name, locations_with, tr, tx,
};
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

/// What a transfer's row needs beyond its columns, and the sheets of its
/// buttons.
#[derive(Serialize, Debug, Clone)]
pub struct TransferSheet {
    id: i64,
    can_send: bool,
    can_receive: bool,
    warning: String,
}

/// What the transfers page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/transfers.html")]
pub(crate) struct TransfersPage {
    tx: Texts,
    rows: renox::grid::GridPage<Transfer>,
    can_make: bool,
    from: Vec<(String, String)>,
    to: Vec<(String, String)>,
    items: Vec<(String, String)>,
    lots: Vec<(String, String)>,
    /// The transfers the person may send or receive (a sheet each).
    sheets: Vec<TransferSheet>,
}

/// The transfers grid: search on the number, the stores and the items, filters
/// by the stores (from, to) and status. The items in words are drawn by the
/// template (`custom`); the hidden `items` and `lots` columns carry the item
/// name and the lot number of the lines (the first by name: the screen's
/// form makes transfers of one line) for the search and the filter. No row link: a transfer has no page of its own; send and receive
/// are in the row.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let place = |key: &str, label: &str, foreign_key: &str| {
        Column::related(key, label, "wl_inventory_locations", foreign_key, "name_en").searchable()
    };
    let grid = Grid::new("inventory-transfers")
        .title(&tr(lang, "transfers.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &tr(lang, "transfers.number"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(place("from", &tr(lang, "transfers.from"), "from_location_id").mobile())
        .column(place("to", &tr(lang, "transfers.to"), "to_location_id").mobile())
        .column(Column::custom("lines", &tr(lang, "transfers.lines")))
        .column(
            Column::text("items", &tr(lang, "stock.item"))
                .computed(
                    "(SELECT MIN(\"wl_inventory_items\".\"name_en\") FROM \"wl_inventory_items\" \
                     WHERE \"wl_inventory_items\".\"id\" IN (SELECT \"item_id\" \
                     FROM \"wl_inventory_transfer_lines\" WHERE \"transfer_id\" = {T}.\"id\"))",
                )
                .hidden()
                .searchable(),
        )
        .column(
            Column::text("lots", &tr(lang, "stock.lot"))
                .computed(
                    "(SELECT MIN(\"wl_inventory_lots\".\"number\") FROM \"wl_inventory_lots\" \
                     WHERE \"wl_inventory_lots\".\"id\" IN (SELECT \"lot_id\" \
                     FROM \"wl_inventory_transfer_lines\" WHERE \"transfer_id\" = {T}.\"id\"))",
                )
                .hidden()
                .searchable(),
        )
        .column(
            Column::select(
                "status",
                &tr(lang, "fields.status"),
                crate::stock_screens::options(
                    lang,
                    "transfer",
                    &["draft", "in_transit", "received", "cancelled"],
                ),
            )
            .badges(&[
                ("draft", "info"),
                ("in_transit", "warning"),
                ("received", "success"),
                ("cancelled", "error"),
            ])
            .mobile(),
        )
        .column(
            Column::datetime("when", &tr(lang, "transfers.when"))
                .computed("COALESCE({T}.\"received_at\", {T}.\"sent_at\", {T}.\"created_at\")")
                .mobile(),
        )
        .column(Column::custom("actions", &tr(lang, "stock.actions")))
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(
            &tr(lang, "transfers.empty"),
            Some(&tr(lang, "transfers.empty_hint")),
        );
    if can_export { grid.exports() } else { grid }
}

/// `GET /stock/transfers`.
async fn transfers(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::STOCK_TRANSFER)
        && !user.has_permission_anywhere(permissions::STOCK_VIEW)
    {
        return Err(Error::Forbidden);
    }
    let can_export = user.has_permission_anywhere(permissions::STOCK_TRANSFER);
    let grid = grid(&lang, can_export);
    let query = Transfer::query();
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    let mut sheets = Vec::new();
    for transfer in page.items() {
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
        let can_send = transfer.status == TransferStatus::Draft
            && user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.from_scope());
        let can_receive = transfer.status == TransferStatus::InTransit
            && user.has_permission_in(permissions::STOCK_TRANSFER, &transfer.to_scope());
        let warning = if can_receive {
            warnings_for(&state.db, transfer.to_location_id, &item_ids)
                .await?
                .join(" ")
        } else {
            String::new()
        };
        extra.insert(
            transfer.id,
            json!({
                "lines": lines.join(", "),
                "can_send": can_send,
                "can_receive": can_receive,
            }),
        );
        if can_send || can_receive {
            sheets.push(TransferSheet {
                id: transfer.id,
                can_send,
                can_receive,
                warning,
            });
        }
    }
    let page = page.extend(|transfer| {
        extra
            .get(&transfer.id)
            .cloned()
            .unwrap_or_else(|| json!({}))
    });
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
        rows: page,
        can_make: !from.is_empty(),
        from,
        to,
        items,
        lots,
        sheets,
    })
    .into_response())
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
