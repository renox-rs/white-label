//! The stock screens: the stock of a store with its lots, one lot with its
//! movements, the holds with the decision of each line, and the pending
//! issues (BL-STK-11, BL-STK-12, BL-STK-18; §5.2).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /stock` | `wl_inventory.stock` | `stock.view` in the location's scope |
//! | `GET /stock/lots/{id}` | `wl_inventory.lot` | `stock.view` somewhere |
//! | `GET /stock/holds` | `wl_inventory.holds` | `stock.hold` or `lots.release` somewhere |
//! | `GET /pending-issues` | `wl_inventory.pending_list` | `stock.adjust` somewhere; a page for a browser, JSON for the rest |
//!
//! The forms and buttons post to the routes that were there first
//! ([`crate::stock_pages`]): placing a hold, deciding a line, clearing a
//! pending issue. A page lists only what the user may see in the scope of the
//! location, and shows a button only for what the user may do there; the
//! route checks again.
//!
//! Decision: a hold placed from a lot or a location page names that lot or
//! location as the record that caused it (the hold's *reference*), since the
//! form has no ticket to name; `recall` is not offered as a reason, a recall
//! opens its own holds.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::{subject_label, today};

use crate::balance;

use crate::item::Item;
use crate::location::{Location, LocationType};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::pending_issue::{self, PendingIssue};
use crate::permissions;
use crate::screens::{dubai_text, quantity_text};
use crate::stock_hold::{HoldLineStatus, HoldStatus, REASONS, StockHold, StockHoldLine, lines_of};
use crate::stock_ui_texts::STOCK_TEXTS;
use crate::texts::t;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
pub(crate) const VIEWS: &[(&str, &str)] = &[
    (
        "wl-inventory/stock.html",
        include_str!("../views/wl-inventory/stock.html"),
    ),
    (
        "wl-inventory/lot.html",
        include_str!("../views/wl-inventory/lot.html"),
    ),
    (
        "wl-inventory/holds.html",
        include_str!("../views/wl-inventory/holds.html"),
    ),
    (
        "wl-inventory/pending.html",
        include_str!("../views/wl-inventory/pending.html"),
    ),
    (
        "wl-inventory/transfers.html",
        include_str!("../views/wl-inventory/transfers.html"),
    ),
    (
        "wl-inventory/orders.html",
        include_str!("../views/wl-inventory/orders.html"),
    ),
    (
        "wl-inventory/receipts.html",
        include_str!("../views/wl-inventory/receipts.html"),
    ),
    (
        "wl-inventory/receipt.html",
        include_str!("../views/wl-inventory/receipt.html"),
    ),
];

/// The texts a template reads as `tx['stock.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The prefix of the keys of these screens' texts.
pub(crate) const PREFIX: &str = "wl_inventory.stock.";

/// Every text of the stock screens in `lang`, by the name a template reads.
pub(crate) fn tx(lang: &Lang) -> Texts {
    STOCK_TEXTS
        .iter()
        .filter_map(|(key, _)| Some((key.strip_prefix(PREFIX)?, t(lang, key))))
        .collect()
}

/// The text `key` (without the prefix) in `lang`.
pub(crate) fn tr(lang: &Lang, key: &str) -> String {
    t(lang, &format!("{PREFIX}{key}"))
}

/// The word for `value` in `group` (`lot`, `hold`, `move`, `reason`...), the
/// value with spaces when the group has none.
pub(crate) fn word(lang: &Lang, group: &str, value: &str) -> String {
    let key = format!("{PREFIX}{group}.{value}");
    let found = t(lang, &key);
    if found == key {
        value.replace('_', " ")
    } else {
        found
    }
}

/// The name of an item in the page's language.
pub(crate) fn item_name(item: &Item, lang: &Lang) -> String {
    if lang.locale.starts_with("ar")
        && let Some(name) = item.name_ar.as_deref().filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    item.name_en.clone()
}

/// The name of a location.
pub(crate) fn location_name(location: &Location) -> String {
    location.name_en.clone()
}

/// `12.5 box`.
pub(crate) fn amount(milli: i64, unit: &str) -> String {
    format!("{} {}", quantity_text(milli), unit)
        .trim()
        .to_owned()
}

/// The locations whose stock `user` may see: those where the permission holds
/// in the location's scope.
pub(crate) async fn locations_with(
    state: &AppState,
    user: &AuthUser,
    permission: &str,
) -> Result<Vec<Location>> {
    let mut found = Vec::new();
    for location in Location::query()
        .order_by(Location::NAME_EN)
        .get(&state.db)
        .await?
    {
        // The place a transfer waits on the road is not a store.
        if location.location_type != LocationType::InTransit
            && user.has_permission_in(permission, &location.scope())
        {
            found.push(location);
        }
    }
    Ok(found)
}

/// A label for a record, the registered one or nothing.
pub(crate) async fn label(state: &AppState, subject: &Subject) -> String {
    subject_label(&state.db, subject)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The badge colour of a lot's status.
fn lot_kind(status: LotStatus) -> &'static str {
    match status {
        LotStatus::Released => "success",
        LotStatus::Quarantine => "warning",
        LotStatus::OnHold | LotStatus::Expired | LotStatus::Rejected => "error",
        LotStatus::Depleted => "info",
    }
}

/// The expiry as text, with the colour that says how near it is.
fn expiry_of(lot: &Lot, lang: &Lang) -> (String, &'static str) {
    match lot.expiry_date {
        None => (tr(lang, "none"), "info"),
        Some(day) => {
            let left = (day - today()).num_days();
            let kind = if left < 0 {
                "error"
            } else if left <= 30 {
                "warning"
            } else {
                "info"
            };
            (day.to_string(), kind)
        }
    }
}

/// What the stock page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/stock.html")]
pub(crate) struct StockPage {
    tx: Texts,
    locations: Vec<(String, String)>,
    location: String,
    location_name: String,
    items: GridPage<Item>,
    lots: GridPage<Lot>,
    nothing: bool,
}

#[derive(serde::Deserialize, Default)]
struct StockQuery {
    #[serde(default)]
    location: String,
}

/// The quantity of the item (or lot) in a store, in thousandths, as SQL: what
/// arrived there less what left. `owner` is the column of the movements that
/// names the row (`item_id` or `lot_id`).
fn on_hand_sql(owner: &str, location: i64) -> String {
    format!(
        "(COALESCE((SELECT SUM(m.\"quantity_milli\") FROM \"wl_inventory_stock_movements\" m \
         WHERE m.\"{owner}\" = {{T}}.\"id\" AND m.\"to_location_id\" = {location}), 0) \
         - COALESCE((SELECT SUM(m.\"quantity_milli\") FROM \"wl_inventory_stock_movements\" m \
         WHERE m.\"{owner}\" = {{T}}.\"id\" AND m.\"from_location_id\" = {location}), 0))"
    )
}

/// The items grid of a store: the items on hand there or with a reorder
/// point, with what is held, reserved and available, and the level.
fn items_grid(lang: &Lang, location: i64) -> Grid {
    let t = tx(lang);
    Grid::new("wl-inventory-stock-items")
        .prefix("items")
        .title(&t["stock.items"])
        .column(
            Column::text("name_en", &t["stock.item"])
                .description("code")
                .searchable()
                .frozen()
                .mobile(),
        )
        .column(Column::text("code", &t["stock.code"]).searchable().hidden())
        .column(
            Column::text("on_hand", &t["stock.on_hand"])
                .computed(&on_hand_sql("item_id", location))
                .filterable(false)
                .mobile(),
        )
        .column(
            Column::text("held", &t["stock.held"])
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::text("reserved", &t["stock.reserved"])
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::text("available", &t["stock.available"])
                .sortable(false)
                .filterable(false)
                .mobile(),
        )
        .column(
            Column::text("reorder", &t["stock.reorder"])
                .computed("{T}.\"reorder_point_milli\"")
                .filterable(false),
        )
        .column(
            Column::select(
                "level",
                &t["stock.level"],
                [
                    ("low", t["stock.low"].clone()),
                    ("ok", t["stock.level_ok"].clone()),
                ],
            )
            .computed(&format!(
                "(CASE WHEN {{T}}.\"reorder_point_milli\" > 0 AND {} <= {{T}}.\"reorder_point_milli\" \
                 THEN 'low' ELSE 'ok' END)",
                on_hand_sql("item_id", location)
            ))
            .badges(&[("low", "warning"), ("ok", "neutral")])
            .mobile(),
        )
        .sort_by("name_en")
        .cards_on_mobile()
        .empty_state(&t["stock.empty"], None)
}

/// The lots grid of a store: the lots with something on hand there.
fn lots_grid(lang: &Lang, location: i64) -> Grid {
    let t = tx(lang);
    Grid::new("wl-inventory-stock-lots")
        .prefix("lots")
        .title(&t["stock.lots"])
        .column(
            Column::text("number", &t["stock.lot"])
                .link("/stock/lots/{id}")
                .searchable()
                .frozen()
                .mobile(),
        )
        .column(
            Column::related(
                "item",
                &t["stock.item"],
                "wl_inventory_items",
                "item_id",
                "name_en",
            )
            .searchable()
            .mobile(),
        )
        .column(Column::text("supplier_lot", &t["stock.supplier_lot"]).searchable())
        .column(Column::date("expiry_date", &t["stock.expiry"]).mobile())
        .column(Column::custom("expiry_flag", "").mobile())
        .column(
            Column::text("here", &t["stock.here"])
                .computed(&on_hand_sql("lot_id", location))
                .filterable(false)
                .mobile(),
        )
        .column(
            Column::text("held", &t["stock.held"])
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::text("reserved", &t["stock.reserved"])
                .sortable(false)
                .filterable(false),
        )
        .column(
            Column::select(
                "status",
                &t["fields.status"],
                [
                    "quarantine",
                    "released",
                    "on_hold",
                    "expired",
                    "rejected",
                    "depleted",
                ]
                .map(|status| (status, word(lang, "lot", status))),
            )
            .badges(&[
                ("released", "success"),
                ("quarantine", "warning"),
                ("on_hold", "danger"),
                ("expired", "danger"),
                ("rejected", "danger"),
                ("depleted", "neutral"),
            ])
            .mobile(),
        )
        .sort_by("expiry_date")
        .row_url("/stock/lots/{id}")
        .cards_on_mobile()
        .empty_state(&t["stock.no_lots"], None)
}

/// `GET /stock`: the items and the lots of one store, the first the user may
/// see unless `?location=` names another. Each is a grid (sort, filter,
/// search, pages) that reads its own names of the query string:
/// `?items.page=2`, `?lots.sort=-here`.
async fn stock(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<StockQuery>,
    request: GridRequest,
) -> Result<View> {
    if !user.has_permission_anywhere(permissions::STOCK_VIEW) {
        return Err(Error::Forbidden);
    }
    let visible = locations_with(&state, &user, permissions::STOCK_VIEW).await?;
    let wanted = query.location.trim().parse::<i64>().ok();
    let chosen = wanted
        .and_then(|id| visible.iter().find(|location| location.id == id))
        .or_else(|| visible.first())
        .cloned();
    let Some(chosen) = chosen else {
        return Ok(View::typed(StockPage {
            tx: tx(&lang),
            locations: Vec::new(),
            location: String::new(),
            location_name: String::new(),
            items: items_grid(&lang, 0)
                .page(Item::query().none(), &request)
                .await?,
            lots: lots_grid(&lang, 0)
                .page(Lot::query().none(), &request)
                .await?,
            nothing: true,
        }));
    };
    let on_hand = on_hand_sql("item_id", chosen.id).replace("{T}", "\"wl_inventory_items\"");
    let items_query = Item::query().where_raw(
        &format!("\"wl_inventory_items\".\"reorder_point_milli\" > 0 OR {on_hand} <> 0"),
        Vec::<i64>::new(),
    );
    let lot_here = on_hand_sql("lot_id", chosen.id).replace("{T}", "\"wl_inventory_lots\"");
    let lots_query = Lot::query().where_raw(&format!("{lot_here} > 0"), Vec::<i64>::new());
    let items_page = items_grid(&lang, chosen.id)
        .page(items_query, &request)
        .await?;
    let mut figures = BTreeMap::new();
    for item in items_page.items() {
        let found = balance::balance(&state.db, item.id, chosen.id, None).await?;
        let reserved =
            crate::reservation::reserved_in(&state.db, item.id, chosen.id, None, None).await?;
        figures.insert(
            item.id,
            renox::serde_json::json!({
                "name_en": item_name(item, &lang),
                "on_hand": amount(found.on_hand_milli, &item.base_unit),
                "held": amount(found.held_milli, &item.base_unit),
                "reserved": amount(reserved, &item.base_unit),
                "available": amount(found.available_milli(), &item.base_unit),
                "reorder": if item.reorder_point_milli > 0 {
                    amount(item.reorder_point_milli, &item.base_unit)
                } else {
                    tr(&lang, "none")
                },
            }),
        );
    }
    let items = items_page.extend(|item| figures.get(&item.id).cloned().unwrap_or_default());
    let lots_page = lots_grid(&lang, chosen.id)
        .page(lots_query, &request)
        .await?;
    let mut lot_figures = BTreeMap::new();
    for lot in lots_page.items() {
        let item = Item::find_or_404(&state.db, lot.item_id).await?;
        let here = balance::balance(&state.db, item.id, chosen.id, Some(lot.id)).await?;
        let reserved =
            crate::reservation::reserved_in(&state.db, item.id, chosen.id, Some(lot.id), None)
                .await?;
        let (expiry, expiry_kind) = expiry_of(lot, &lang);
        lot_figures.insert(
            lot.id,
            renox::serde_json::json!({
                "item": item_name(&item, &lang),
                "here": amount(here.on_hand_milli, &item.base_unit),
                "held": amount(here.held_milli, &item.base_unit),
                "reserved": amount(reserved, &item.base_unit),
                "expiry": expiry,
                "expiry_kind": expiry_kind,
            }),
        );
    }
    let lots = lots_page.extend(|lot| lot_figures.get(&lot.id).cloned().unwrap_or_default());
    Ok(View::typed(StockPage {
        tx: tx(&lang),
        locations: visible
            .iter()
            .map(|location| (location.id.to_string(), location_name(location)))
            .collect(),
        location: chosen.id.to_string(),
        location_name: location_name(&chosen),
        nothing: items.total() == 0,
        items,
        lots,
    }))
}

/// One place a lot is at.
#[derive(Serialize, Debug, Clone)]
pub struct WhereRow {
    location: String,
    quantity: String,
    reserved: String,
}

/// One movement of a lot.
#[derive(Serialize, Debug, Clone)]
pub struct MoveRow {
    when: String,
    kind: String,
    from: String,
    to: String,
    quantity: String,
    source: String,
    by: String,
}

/// One hold that covers a lot.
#[derive(Serialize, Debug, Clone)]
pub struct LotHoldRow {
    number: String,
    reason: String,
    status: String,
    status_kind: &'static str,
}

/// What the lot page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/lot.html")]
pub(crate) struct LotPage {
    tx: Texts,
    id: i64,
    title: String,
    item: String,
    number: String,
    supplier_lot: String,
    expiry: String,
    expiry_kind: &'static str,
    status: String,
    status_kind: &'static str,
    unit: String,
    places: Vec<WhereRow>,
    movements: Vec<MoveRow>,
    holds: Vec<LotHoldRow>,
    can_hold: bool,
    reasons: Vec<(String, String)>,
    reference: String,
    back: String,
}

/// The reasons a hold can be placed for by hand.
pub(crate) fn hold_reasons(lang: &Lang) -> Vec<(String, String)> {
    REASONS
        .iter()
        .filter(|reason| **reason != "recall")
        .map(|reason| ((*reason).to_owned(), word(lang, "reason", reason)))
        .collect()
}

/// The status of a hold, as a word and a colour.
fn hold_status(lang: &Lang, status: HoldStatus) -> (String, &'static str) {
    (
        word(lang, "hold", status.as_str()),
        match status {
            HoldStatus::Open => "warning",
            HoldStatus::Closed => "success",
            HoldStatus::Cancelled => "info",
        },
    )
}

/// `GET /stock/lots/{id}`: the lot, where it is and every movement of it.
async fn lot(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !user.has_permission_anywhere(permissions::STOCK_VIEW) {
        return Err(Error::Forbidden);
    }
    let lot = Lot::find_or_404(&state.db, id).await?;
    let item = Item::find_or_404(&state.db, lot.item_id).await?;
    let mut places = Vec::new();
    for (location_id, quantity) in balance::lot_locations(&state.db, lot.id).await? {
        if let Some(location) = Location::find(&state.db, location_id).await?
            && user.has_permission_in(permissions::STOCK_VIEW, &location.scope())
        {
            places.push(WhereRow {
                location: location_name(&location),
                quantity: amount(quantity, &item.base_unit),
                reserved: amount(
                    crate::reservation::reserved_in(
                        &state.db,
                        item.id,
                        location.id,
                        Some(lot.id),
                        None,
                    )
                    .await?,
                    &item.base_unit,
                ),
            });
        }
    }
    let mut movements = Vec::new();
    for movement in StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot.id))
        .order_by_desc(StockMovement::ID)
        .limit(200)
        .get(&state.db)
        .await?
    {
        movements.push(move_row(&state, &lang, &item, &movement).await?);
    }
    let mut holds = Vec::new();
    let mut seen = Vec::new();
    for line in StockHoldLine::query()
        .where_(StockHoldLine::LOT_ID.eq(Some(lot.id)))
        .order_by_desc(StockHoldLine::ID)
        .get(&state.db)
        .await?
    {
        if seen.contains(&line.stock_hold_id) {
            continue;
        }
        seen.push(line.stock_hold_id);
        let hold = StockHold::find_or_404(&state.db, line.stock_hold_id).await?;
        let (status, status_kind) = hold_status(&lang, hold.status);
        holds.push(LotHoldRow {
            number: hold.number.clone().unwrap_or_default(),
            reason: word(&lang, "reason", &hold.reason),
            status,
            status_kind,
        });
    }
    let (expiry, expiry_kind) = expiry_of(&lot, &lang);
    let holdable = matches!(lot.status, LotStatus::Released | LotStatus::Quarantine);
    Ok(View::typed(LotPage {
        tx: tx(&lang),
        id: lot.id,
        title: format!("{} · {}", item_name(&item, &lang), lot.number),
        item: item_name(&item, &lang),
        number: lot.number.clone(),
        supplier_lot: lot.supplier_lot.clone().unwrap_or_default(),
        expiry,
        expiry_kind,
        status: word(&lang, "lot", lot.status.as_str()),
        status_kind: lot_kind(lot.status),
        unit: item.base_unit.clone(),
        places,
        movements,
        holds,
        can_hold: holdable && user.has_permission(permissions::STOCK_HOLD),
        reasons: hold_reasons(&lang),
        reference: format!("{}:{}", wl_kernel::inventory::LOT_SUBJECT_TYPE, lot.id),
        back: "/stock".to_owned(),
    }))
}

/// A movement as the lot page lists it.
async fn move_row(
    state: &AppState,
    lang: &Lang,
    item: &Item,
    movement: &StockMovement,
) -> Result<MoveRow> {
    let place = |id: Option<i64>| async move {
        match id {
            Some(id) => Ok::<_, Error>(
                Location::find(&state.db, id)
                    .await?
                    .map(|location| location_name(&location))
                    .unwrap_or_default(),
            ),
            None => Ok(String::new()),
        }
    };
    let by = match movement.user_id {
        Some(id) => renox::auth::User::find(&state.db, id)
            .await?
            .map(|user| user.name)
            .unwrap_or_default(),
        None => String::new(),
    };
    let signed = match movement.movement_type {
        MovementType::Issue
        | MovementType::Consume
        | MovementType::TransferOut
        | MovementType::Wastage
        | MovementType::ReturnOut
        | MovementType::Dispatch => {
            format!("-{}", amount(movement.quantity_milli, &item.base_unit))
        }
        _ => format!("+{}", amount(movement.quantity_milli, &item.base_unit)),
    };
    Ok(MoveRow {
        when: dubai_text(movement.moved_at),
        kind: word(lang, "move", movement.movement_type.as_str()),
        from: place(movement.from_location_id).await?,
        to: place(movement.to_location_id).await?,
        quantity: signed,
        source: label(state, &movement.source()).await,
        by,
    })
}

/// One line of a hold.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    id: i64,
    item: String,
    lot_url: String,
    lot: String,
    location: String,
    quantity: String,
    status: String,
    status_kind: &'static str,
    waiting: bool,
    can_decide: bool,
}

/// What the page adds to a hold: its lines with the buttons of the decision,
/// the reference in words and whether it can be cancelled.
struct HoldExtra {
    reference: String,
    lines: Vec<LineRow>,
    can_cancel: bool,
}

/// What the holds page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/holds.html")]
pub(crate) struct HoldsPage {
    tx: Texts,
    holds: GridPage<StockHold>,
    approvals: Vec<ApprovalCard>,
    can_hold: bool,
    locations: Vec<(String, String)>,
    reasons: Vec<(String, String)>,
}

/// The holds grid, the open ones first: number, reason, what caused it, when
/// it was placed, status and lines. A click opens the lines with the
/// decision of each; the last column has the cancel button.
fn holds_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    Grid::new("wl-inventory-holds")
        .title(&t["holds.title"])
        .column(
            Column::text("number", &t["hold.number"])
                .searchable()
                .frozen()
                .mobile(),
        )
        .column(
            Column::select(
                "reason",
                &t["hold.reason"],
                REASONS.map(|reason| (reason, word(lang, "reason", reason))),
            )
            .mobile(),
        )
        .column(
            Column::text("reference", &t["holds.reference"])
                .sortable(false)
                .filterable(false),
        )
        .column(Column::datetime("placed_at", &t["holds.placed"]))
        .column(
            Column::select(
                "status",
                &t["fields.status"],
                ["open", "closed", "cancelled"].map(|status| (status, word(lang, "hold", status))),
            )
            .badges(&[
                ("open", "warning"),
                ("closed", "success"),
                ("cancelled", "neutral"),
            ])
            .mobile(),
        )
        .column(Column::count_of(
            "line_count",
            &t["holds.lines"],
            "wl_inventory_stock_hold_lines",
            "stock_hold_id",
        ))
        .column(
            Column::number("open_first", "")
                .computed("(CASE WHEN {T}.\"status\" = 'open' THEN 0 ELSE 1 END)")
                .hidden(),
        )
        .column(Column::custom("actions", "").frozen_right().mobile())
        .sort_by("open_first,-id")
        .details()
        .cards_on_mobile()
        .empty_state(&t["holds.empty"], None)
}

/// `GET /stock/holds`: the open holds first, each with its lines and the
/// buttons of the decision the user may take; a decision that waits for a
/// second person shows its approval card.
async fn holds(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    let decider = user.has_permission_anywhere(permissions::LOTS_RELEASE);
    let placer = user.has_permission_anywhere(permissions::STOCK_HOLD);
    if !decider && !placer {
        return Err(Error::Forbidden);
    }
    let page = holds_grid(&lang).page(StockHold::query(), &request).await?;
    let mut extras = BTreeMap::new();
    let mut cards = Vec::new();
    for hold in page.items() {
        let mut lines = Vec::new();
        let raw = lines_of(&state.db, hold.id).await?;
        let all_undecided = raw
            .iter()
            .all(|line| line.status == HoldLineStatus::Undecided);
        for line in raw {
            let item = Item::find_or_404(&state.db, line.item_id).await?;
            let location = Location::find_or_404(&state.db, line.location_id).await?;
            let lot = match line.lot_id {
                Some(id) => Lot::find(&state.db, id).await?,
                None => None,
            };
            let pending = approvals::pending_for(&state.db, &line.subject()).await?;
            for request in &pending {
                cards.push(approvals::card(&state.db, request, &user, &lang).await?);
            }
            let (status, status_kind) = match line.status {
                HoldLineStatus::Undecided => (word(&lang, "line", "undecided"), "warning"),
                HoldLineStatus::Released => (word(&lang, "line", "released"), "success"),
                HoldLineStatus::Discarded => (word(&lang, "line", "discarded"), "error"),
                HoldLineStatus::ReturnedToSupplier => {
                    (word(&lang, "line", "returned_to_supplier"), "info")
                }
            };
            lines.push(LineRow {
                id: line.id,
                item: item_name(&item, &lang),
                lot_url: lot
                    .as_ref()
                    .map(|lot| format!("/stock/lots/{}", lot.id))
                    .unwrap_or_default(),
                lot: lot.map(|lot| lot.number).unwrap_or_default(),
                location: location_name(&location),
                quantity: amount(line.quantity_milli, &item.base_unit),
                status,
                status_kind,
                waiting: !pending.is_empty(),
                can_decide: hold.status == HoldStatus::Open
                    && line.status == HoldLineStatus::Undecided
                    && pending.is_empty()
                    && user.has_permission_in(permissions::LOTS_RELEASE, &line.scope()),
            });
        }
        let reference = match hold.reference() {
            Some(subject) => label(&state, &subject).await,
            None => String::new(),
        };
        let can_cancel = hold.status == HoldStatus::Open
            && hold.reason != "recall"
            && all_undecided
            && user.has_permission_in(permissions::LOTS_RELEASE, &hold.scope());
        extras.insert(
            hold.id,
            HoldExtra {
                reference,
                lines,
                can_cancel,
            },
        );
    }
    let holds = page.extend(|hold| match extras.get(&hold.id) {
        Some(extra) => renox::serde_json::json!({
            "reference": extra.reference,
            "lines": extra.lines,
            "can_cancel": extra.can_cancel,
        }),
        None => renox::serde_json::json!({}),
    });
    let locations = locations_with(&state, &user, permissions::STOCK_HOLD)
        .await?
        .iter()
        .map(|location| (location.id.to_string(), location_name(location)))
        .collect::<Vec<_>>();
    Ok(View::typed(HoldsPage {
        tx: tx(&lang),
        holds,
        approvals: cards,
        can_hold: !locations.is_empty(),
        locations,
        reasons: hold_reasons(&lang),
    }))
}

/// One open pending issue.
#[derive(Serialize, Debug, Clone)]
pub struct PendingScreenRow {
    id: i64,
    item: String,
    location: String,
    quantity: String,
    source: String,
    recorded: String,
    lot_tracked: bool,
    lots: Vec<(String, String)>,
    waiting: bool,
    can_clear: bool,
}

/// What the pending issues page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/pending.html")]
pub(crate) struct PendingPage {
    tx: Texts,
    rows: Vec<PendingScreenRow>,
    approvals: Vec<ApprovalCard>,
}

/// The pending issues page: the open ones at the locations where the user may
/// clear them, oldest first, each with the form that clears it (a reason and,
/// for a lot-tracked item, the lot the stock is entered in).
pub(crate) async fn pending_page(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
) -> Result<Response> {
    let mut rows = Vec::new();
    let mut cards = Vec::new();
    for pending in pending_issue::open_of(&state.db).await? {
        if !user.has_permission_in(permissions::STOCK_ADJUST, &pending.scope()) {
            continue;
        }
        rows.push(pending_row(state, user, lang, &pending, &mut cards).await?);
    }
    Ok(View::typed(PendingPage {
        tx: tx(lang),
        rows,
        approvals: cards,
    })
    .into_response())
}

async fn pending_row(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    pending: &PendingIssue,
    cards: &mut Vec<ApprovalCard>,
) -> Result<PendingScreenRow> {
    let item = Item::find_or_404(&state.db, pending.item_id).await?;
    let location = Location::find_or_404(&state.db, pending.location_id).await?;
    let mut lots = Vec::new();
    if item.lot_tracked {
        for lot in Lot::query()
            .where_(Lot::ITEM_ID.eq(item.id))
            .order_by(Lot::EXPIRY_DATE)
            .get(&state.db)
            .await?
        {
            if !matches!(lot.status, LotStatus::Released | LotStatus::Quarantine) {
                continue;
            }
            let expiry = lot
                .expiry_date
                .map(|day| format!(" ({day})"))
                .unwrap_or_default();
            lots.push((lot.id.to_string(), format!("{}{}", lot.number, expiry)));
        }
    }
    let requests = approvals::pending_for(&state.db, &pending.subject()).await?;
    for request in &requests {
        cards.push(approvals::card(&state.db, request, user, lang).await?);
    }
    Ok(PendingScreenRow {
        id: pending.id,
        item: item_name(&item, lang),
        location: location_name(&location),
        quantity: amount(pending.quantity_milli, &item.base_unit),
        source: label(state, &pending.source()).await,
        recorded: dubai_text(pending.recorded_at),
        lot_tracked: item.lot_tracked,
        lots,
        waiting: !requests.is_empty(),
        can_clear: requests.is_empty(),
    })
}

/// The routes of the stock screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/stock", stock)
        .name("wl_inventory.stock")
        .get("/stock/lots/{id}", lot)
        .name("wl_inventory.lot")
        .get("/stock/holds", holds)
        .name("wl_inventory.holds")
        .post("/stock/holds", place)
        .name("wl_inventory.hold_screen_place")
        .require_auth()
}

/// What a user enters to place a hold from a lot's page or the holds page.
#[derive(serde::Deserialize, Serialize, Validate, Debug, Default)]
pub struct PlaceForm {
    /// One of the hold reasons, not `recall`.
    #[validate(max = 32)]
    pub reason: Option<String>,
    /// The id of the lot to hold, wherever it is.
    #[validate(max = 20)]
    pub lot: Option<String>,
    /// The id of the store to hold.
    #[validate(max = 20)]
    pub location: Option<String>,
}

fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `POST /stock/holds`: places a hold on a lot or on a store. The lot or the
/// store is the record the hold names as its reference.
async fn place(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<PlaceForm>,
) -> Result<Response> {
    use crate::hold_service::{NewHold, NewScope, place_as};
    let id = |field: &Option<String>| {
        field
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<i64>().ok())
    };
    let reason = form.reason.clone().unwrap_or_default();
    if reason == "recall" || !REASONS.contains(&reason.as_str()) {
        return Err(refused(tr(&lang, "problem.reason")));
    }
    let (scope, reference) = match (id(&form.lot), id(&form.location)) {
        (Some(lot), None) => (
            NewScope::Lots(vec![lot]),
            Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot),
        ),
        (None, Some(location)) => (
            NewScope::Location {
                location_id: location,
                item_ids: Vec::new(),
            },
            Subject::new(wl_kernel::inventory::LOCATION_SUBJECT_TYPE, location),
        ),
        _ => return Err(refused(tr(&lang, "problem.hold_what"))),
    };
    place_as(
        &state,
        &user,
        NewHold {
            reason,
            reference,
            scope,
            by: None,
        },
    )
    .await?;
    Ok((Toast::success(tr(&lang, "hold_placed")), renox::HxRefresh).into_response())
}

/// The labels of the records these screens show by number.
pub(crate) fn register_labels() {
    use wl_kernel::documents::register_subject_label as register;
    use wl_kernel::inventory::{
        GOODS_RECEIPT_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
        PENDING_ISSUE_SUBJECT_TYPE, PURCHASE_ORDER_SUBJECT_TYPE, RECALL_SUBJECT_TYPE,
        STOCK_HOLD_LINE_SUBJECT_TYPE, STOCK_HOLD_SUBJECT_TYPE, TRANSFER_SUBJECT_TYPE,
    };
    // The number alone: a page that names a lot (a recall's trace) never
    // names the product.
    // The code and the name: a page that names an item (a certificate that is
    // missing for it) says which.
    register(wl_kernel::inventory::ITEM_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(Item::find(db, id)
                .await?
                .map(|item| format!("{} - {}", item.code, item.name_en)))
        })
    });
    register(LOT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Lot::find(db, id).await?.map(|lot| lot.number)) })
    });
    register(LOCATION_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Location::find(db, id).await?.map(|row| row.name_en)) })
    });
    register(STOCK_HOLD_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(StockHold::find(db, id).await?.and_then(|row| row.number)) })
    });
    register(RECALL_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(crate::recall::Recall::find(db, id)
                .await?
                .and_then(|row| row.number))
        })
    });
    register(STOCK_HOLD_LINE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            let Some(line) = StockHoldLine::find(db, id).await? else {
                return Ok(None);
            };
            let hold = StockHold::find(db, line.stock_hold_id).await?;
            let item = Item::find(db, line.item_id).await?;
            Ok(Some(format!(
                "{} · {}",
                hold.and_then(|hold| hold.number).unwrap_or_default(),
                item.map(|item| item.name_en).unwrap_or_default()
            )))
        })
    });
    register(TRANSFER_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(crate::transfer::Transfer::find(db, id)
                .await?
                .and_then(|row| row.number))
        })
    });
    register(GOODS_RECEIPT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(crate::goods_receipt::GoodsReceipt::find(db, id)
                .await?
                .map(|row| row.number.unwrap_or_else(|| format!("GRN #{}", row.id))))
        })
    });
    register(PURCHASE_ORDER_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(crate::purchase_order::PurchaseOrder::find(db, id)
                .await?
                .and_then(|row| row.number))
        })
    });
    register(PENDING_ISSUE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            let Some(pending) = PendingIssue::find(db, id).await? else {
                return Ok(None);
            };
            let item = Item::find(db, pending.item_id).await?;
            Ok(item.map(|item| item.name_en))
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tx['name']` a template reads is a text of the crate: a missing
    /// one would show as a raw key on the page.
    #[test]
    fn every_text_a_template_reads_exists() {
        for (name, source) in VIEWS.iter().chain(crate::recall_screens::VIEWS) {
            let mut rest = *source;
            while let Some(at) = rest.find("tx['") {
                rest = &rest[at + 4..];
                let end = rest.find('\'').expect("a closed key");
                let key = format!("{PREFIX}{}", &rest[..end]);
                assert!(
                    STOCK_TEXTS.iter().any(|(known, _)| *known == key),
                    "{name} reads `{key}`, which has no text"
                );
            }
        }
    }
}
