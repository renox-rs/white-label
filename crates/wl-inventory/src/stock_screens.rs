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

use renox::grid::{Column, Grid};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::{dubai_text, subject_label, today};

use crate::balance;

use crate::item::Item;
use crate::location::{Location, LocationType};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::pending_issue::{self, PendingIssue};
use crate::permissions;
use crate::screens::quantity_text;
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

/// The grid the stock page shows: the items of the store or its lots.
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum StockRows {
    Items(renox::grid::GridPage<Item>),
    Lots(renox::grid::GridPage<Lot>),
}

/// What the stock page needs: the store picker, the switches between the
/// items and the lots, and the grid.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/stock.html")]
pub(crate) struct StockPage {
    tx: Texts,
    locations: Vec<(String, String)>,
    location: String,
    location_name: String,
    items_url: String,
    lots_url: String,
    low_url: String,
    lots_view: bool,
    low_only: bool,
    rows: StockRows,
}

#[derive(serde::Deserialize, Default)]
struct StockQuery {
    #[serde(default)]
    location: String,
    #[serde(default)]
    view: String,
    #[serde(default)]
    level: String,
}

/// The options of a select column: `values` with their words from `group`.
pub(crate) fn options(lang: &Lang, group: &str, values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| ((*value).to_owned(), word(lang, group, value)))
        .collect()
}

/// The export file when the request asks for one; refused for a person who
/// may not export.
pub(crate) async fn exported<M: Model + Serialize>(
    grid: &renox::grid::Grid,
    query: &renox::db::Query<M>,
    request: &renox::grid::GridRequest,
    can_export: bool,
) -> Result<Option<Response>> {
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    grid.export(query.clone(), request).await
}

/// `ids`, or `-1` when none: a query on no row.
pub(crate) fn ids_or_none(mut ids: Vec<i64>) -> Vec<i64> {
    if ids.is_empty() {
        ids.push(-1);
    }
    ids
}

/// The items grid of the stock page: search on the code and the name,
/// quantities in the item's unit. The figures of the store are worked out per
/// item and added to the rows, so they are not sorted or filtered by the grid.
pub(crate) fn items_grid(lang: &Lang) -> Grid {
    let quantity = |key: &str, label: &str| {
        Column::quantity(key, label)
            .scale(3)
            .unit_from("base_unit")
            .sortable(false)
            .filterable(false)
    };
    Grid::new("inventory-stock")
        .title(&tr(lang, "stock.items"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("code", &tr(lang, "stock.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(
            Column::text("name_en", &tr(lang, "stock.item"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("name_ar", &tr(lang, "stock.item_ar")).hidden().searchable())
        .column(quantity("on_hand_milli", &tr(lang, "stock.on_hand")).mobile())
        .column(quantity("held_milli", &tr(lang, "stock.held")).hidden())
        .column(quantity("available_milli", &tr(lang, "stock.available")).mobile())
        .column(
            Column::quantity("reorder_point_milli", &tr(lang, "stock.reorder"))
                .scale(3)
                .unit_from("base_unit")
                .hidden(),
        )
        .column(Column::custom("level", &tr(lang, "stock.level")).mobile())
        .sort_by("name_en")
        .cards_on_mobile()
        .empty_state(&tr(lang, "stock.empty"), Some(&tr(lang, "stock.empty_hint")))
}

/// The lots grid of the stock page: the lots of lot-tracked items with stock
/// in the store, by expiry and status.
pub(crate) fn lots_grid(lang: &Lang) -> Grid {
    let quantity = |key: &str, label: &str| {
        Column::quantity(key, label)
            .scale(3)
            .unit_from("unit")
            .sortable(false)
            .filterable(false)
    };
    Grid::new("inventory-stock-lots")
        .title(&tr(lang, "stock.lots"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &tr(lang, "stock.lot"))
                .frozen()
                .mobile()
                .searchable()
                .link("/stock/lots/{id}"),
        )
        .column(
            Column::related(
                "item",
                &tr(lang, "stock.item"),
                "wl_inventory_items",
                "item_id",
                "name_en",
            )
            .mobile()
            .searchable(),
        )
        .column(
            Column::text("supplier_lot", &tr(lang, "stock.supplier_lot"))
                .hidden()
                .searchable(),
        )
        .column(Column::date("expiry_date", &tr(lang, "stock.expiry_date")).hidden())
        .column(Column::custom("expiry", &tr(lang, "stock.expiry")).mobile())
        .column(quantity("here_milli", &tr(lang, "stock.here")).mobile())
        .column(quantity("held_milli", &tr(lang, "stock.held")).hidden())
        .column(
            Column::select(
                "status",
                &tr(lang, "fields.status"),
                options(
                    lang,
                    "lot",
                    &[
                        "quarantine",
                        "released",
                        "on_hold",
                        "expired",
                        "rejected",
                        "depleted",
                    ],
                ),
            )
            .badges(&[
                ("released", "success"),
                ("quarantine", "warning"),
                ("on_hold", "error"),
                ("expired", "error"),
                ("rejected", "error"),
                ("depleted", "info"),
            ])
            .mobile(),
        )
        .sort_by("expiry_date")
        .cards_on_mobile()
        .empty_state(&tr(lang, "stock.no_lots"), Some(&tr(lang, "stock.no_lots_hint")))
}

/// `GET /stock`: the items of one store (`?view=lots` for its lots,
/// `?level=low` for the items at or below their reorder point), the first
/// store the user may see unless `?location=` names another.
async fn stock(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<StockQuery>,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    if !user.has_permission_anywhere(permissions::STOCK_VIEW) {
        return Err(Error::Forbidden);
    }
    let visible = locations_with(&state, &user, permissions::STOCK_VIEW).await?;
    let wanted = query.location.trim().parse::<i64>().ok();
    let chosen = wanted
        .and_then(|id| visible.iter().find(|location| location.id == id))
        .or_else(|| visible.first())
        .cloned();
    let lots_view = query.view == "lots";
    let low_only = query.level == "low" && !lots_view;
    let here = match &chosen {
        Some(location) => format!("/stock?location={}", location.id),
        None => "/stock".to_owned(),
    };
    let separator = if here.contains('?') { '&' } else { '?' };
    let page_of = |rows: StockRows| StockPage {
        tx: tx(&lang),
        locations: visible
            .iter()
            .map(|location| (location.id.to_string(), location_name(location)))
            .collect(),
        location: chosen
            .as_ref()
            .map(|location| location.id.to_string())
            .unwrap_or_default(),
        location_name: chosen.as_ref().map(location_name).unwrap_or_default(),
        items_url: here.clone(),
        lots_url: format!("{here}{separator}view=lots"),
        low_url: format!("{here}{separator}level=low"),
        lots_view,
        low_only,
        rows,
    };
    let all_items = Item::query().order_by(Item::NAME_EN).get(&state.db).await?;
    if lots_view {
        let mut ids = Vec::new();
        let mut figures = std::collections::HashMap::new();
        if let Some(chosen) = &chosen {
            for item in all_items.iter().filter(|item| item.lot_tracked) {
                for lot in Lot::query()
                    .where_(Lot::ITEM_ID.eq(item.id))
                    .get(&state.db)
                    .await?
                {
                    let found =
                        balance::balance(&state.db, item.id, chosen.id, Some(lot.id)).await?;
                    if found.on_hand_milli <= 0 {
                        continue;
                    }
                    ids.push(lot.id);
                    figures.insert(lot.id, (found, item.clone()));
                }
            }
        }
        let grid = lots_grid(&lang);
        let query = Lot::query().where_(Lot::ID.is_in(ids_or_none(ids)));
        let page = grid.page(query, &request).await?.extend(|lot| {
            let Some((found, item)) = figures.get(&lot.id) else {
                return json!({});
            };
            let (expiry, expiry_kind) = expiry_of(lot, &lang);
            json!({
                "here_milli": found.on_hand_milli,
                "held_milli": found.held_milli,
                "unit": item.base_unit,
                "item": item_name(item, &lang),
                "expiry": expiry,
                "expiry_kind": expiry_kind,
            })
        });
        return Ok(View::typed(page_of(StockRows::Lots(page))).into_response());
    }
    let mut ids = Vec::new();
    let mut figures = std::collections::HashMap::new();
    if let Some(chosen) = &chosen {
        for item in &all_items {
            let found = balance::balance(&state.db, item.id, chosen.id, None).await?;
            if found.on_hand_milli == 0 && item.reorder_point_milli == 0 {
                continue;
            }
            let low = item.reorder_point_milli > 0 && found.on_hand_milli <= item.reorder_point_milli;
            if low_only && !low {
                continue;
            }
            ids.push(item.id);
            figures.insert(item.id, (found, low));
        }
    }
    let grid = items_grid(&lang);
    let query = Item::query().where_(Item::ID.is_in(ids_or_none(ids)));
    let page = grid.page(query, &request).await?.extend(|item| {
        let Some((found, low)) = figures.get(&item.id) else {
            return json!({});
        };
        json!({
            "name_en": item_name(item, &lang),
            "on_hand_milli": found.on_hand_milli,
            "held_milli": found.held_milli,
            "available_milli": found.available_milli(),
            "low": low,
        })
    });
    Ok(View::typed(page_of(StockRows::Items(page))).into_response())
}

/// One place a lot is at.
#[derive(Serialize, Debug, Clone)]
pub struct WhereRow {
    location: String,
    quantity: String,
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

/// The holds grid: one row per line of a hold, with the hold's number, reason
/// and status beside it. Filter or group by reason; the decision buttons are
/// drawn by the template (`custom`).
pub(crate) fn holds_grid(lang: &Lang, can_export: bool) -> Grid {
    let of_hold = |column: &str| {
        format!(
            "(SELECT \"{column}\" FROM \"wl_inventory_stock_holds\" WHERE \"id\" = {{T}}.\"stock_hold_id\")"
        )
    };
    let grid = Grid::new("inventory-holds")
        .title(&tr(lang, "holds.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "hold",
                &tr(lang, "hold.number"),
                "wl_inventory_stock_holds",
                "stock_hold_id",
                "number",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(
            Column::select("reason", &tr(lang, "hold.reason"), options(lang, "reason", &REASONS))
                .computed(&of_hold("reason"))
                .mobile(),
        )
        .column(
            Column::select(
                "hold_status",
                &tr(lang, "holds.hold_status"),
                options(lang, "hold", &["open", "closed", "cancelled"]),
            )
            .computed(&of_hold("status"))
            .badges(&[("open", "warning"), ("closed", "success"), ("cancelled", "info")])
            .hidden(),
        )
        .column(Column::datetime("placed_at", &tr(lang, "holds.placed")).computed(&of_hold("placed_at")).hidden())
        .column(Column::custom("reference", &tr(lang, "holds.reference")).hidden())
        .column(
            Column::related(
                "item",
                &tr(lang, "stock.item"),
                "wl_inventory_items",
                "item_id",
                "name_en",
            )
            .mobile()
            .searchable(),
        )
        .column(Column::custom("lot", &tr(lang, "stock.lot")))
        .column(
            Column::related(
                "lot_number",
                &tr(lang, "holds.lot_number"),
                "wl_inventory_lots",
                "lot_id",
                "number",
            )
            .hidden()
            .searchable(),
        )
        .column(
            Column::related(
                "location",
                &tr(lang, "stock.store"),
                "wl_inventory_locations",
                "location_id",
                "name_en",
            )
            .searchable(),
        )
        .column(
            Column::quantity("quantity_milli", &tr(lang, "holds.quantity"))
                .scale(3)
                .unit_from("unit")
                .mobile(),
        )
        .column(
            Column::related(
                "unit",
                &tr(lang, "stock.unit"),
                "wl_inventory_items",
                "item_id",
                "base_unit",
            )
            .hidden(),
        )
        .column(
            Column::select(
                "status",
                &tr(lang, "fields.status"),
                options(
                    lang,
                    "line",
                    &["undecided", "released", "discarded", "returned_to_supplier"],
                ),
            )
            .badges(&[
                ("undecided", "warning"),
                ("released", "success"),
                ("discarded", "error"),
                ("returned_to_supplier", "info"),
            ])
            .mobile(),
        )
        .column(Column::custom("waiting", &tr(lang, "holds.waiting")))
        .column(Column::custom("actions", &tr(lang, "holds.decision")))
        .groups(&["reason", "hold_status"])
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&tr(lang, "holds.empty"), Some(&tr(lang, "holds.empty_hint")));
    if can_export { grid.exports() } else { grid }
}

/// What the holds page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/holds.html")]
pub(crate) struct HoldsPage {
    tx: Texts,
    rows: renox::grid::GridPage<StockHoldLine>,
    approvals: Vec<ApprovalCard>,
    can_hold: bool,
    locations: Vec<(String, String)>,
    reasons: Vec<(String, String)>,
    /// The lines the person may decide (a sheet each).
    deciding: Vec<i64>,
    /// The holds the person may cancel (a sheet each).
    cancellable: Vec<i64>,
}

/// `GET /stock/holds`: the lines of every hold with the buttons of the
/// decision the user may take; a decision that waits for a second person
/// shows its approval card.
async fn holds(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let decider = user.has_permission_anywhere(permissions::LOTS_RELEASE);
    let placer = user.has_permission_anywhere(permissions::STOCK_HOLD);
    if !decider && !placer {
        return Err(Error::Forbidden);
    }
    let grid = holds_grid(&lang, true);
    let query = StockHoldLine::query();
    if let Some(file) = exported(&grid, &query, &request, true).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut cards = Vec::new();
    let mut extra: std::collections::HashMap<i64, renox::serde_json::Value> =
        std::collections::HashMap::new();
    let mut deciding = Vec::new();
    let mut cancellable: Vec<i64> = Vec::new();
    let mut holds: std::collections::HashMap<i64, (StockHold, bool)> =
        std::collections::HashMap::new();
    for line in page.items() {
        if !holds.contains_key(&line.stock_hold_id) {
            let hold = StockHold::find_or_404(&state.db, line.stock_hold_id).await?;
            let all_undecided = lines_of(&state.db, hold.id)
                .await?
                .iter()
                .all(|other| other.status == HoldLineStatus::Undecided);
            holds.insert(hold.id, (hold, all_undecided));
        }
        let (hold, all_undecided) = &holds[&line.stock_hold_id];
        let item = Item::find_or_404(&state.db, line.item_id).await?;
        let lot = match line.lot_id {
            Some(id) => Lot::find(&state.db, id).await?,
            None => None,
        };
        let pending = approvals::pending_for(&state.db, &line.subject()).await?;
        for request in &pending {
            cards.push(approvals::card(&state.db, request, &user, &lang).await?);
        }
        let can_decide = hold.status == HoldStatus::Open
            && line.status == HoldLineStatus::Undecided
            && pending.is_empty()
            && user.has_permission_in(permissions::LOTS_RELEASE, &line.scope());
        let can_cancel = hold.status == HoldStatus::Open
            && hold.reason != "recall"
            && *all_undecided
            && user.has_permission_in(permissions::LOTS_RELEASE, &hold.scope());
        if can_decide {
            deciding.push(line.id);
        }
        if can_cancel && !cancellable.contains(&hold.id) {
            cancellable.push(hold.id);
        }
        let reference = match hold.reference() {
            Some(subject) => label(&state, &subject).await,
            None => String::new(),
        };
        extra.insert(
            line.id,
            json!({
                "item": item_name(&item, &lang),
                "lot_url": lot.as_ref().map(|lot| format!("/stock/lots/{}", lot.id)),
                "lot_number": lot.map(|lot| lot.number),
                "waiting": !pending.is_empty(),
                "can_decide": can_decide,
                "can_cancel": can_cancel,
                "hold_id": hold.id,
                "reference": reference,
            }),
        );
    }
    let page = page.extend(|line| extra.get(&line.id).cloned().unwrap_or_else(|| json!({})));
    let locations = locations_with(&state, &user, permissions::STOCK_HOLD)
        .await?
        .iter()
        .map(|location| (location.id.to_string(), location_name(location)))
        .collect::<Vec<_>>();
    Ok(View::typed(HoldsPage {
        tx: tx(&lang),
        rows: page,
        approvals: cards,
        can_hold: !locations.is_empty(),
        locations,
        reasons: hold_reasons(&lang),
        deciding,
        cancellable,
    })
    .into_response())
}

/// The sheet that clears one pending issue.
#[derive(Serialize, Debug, Clone)]
pub struct PendingSheet {
    id: i64,
    lot_tracked: bool,
    lots: Vec<(String, String)>,
}

/// The pending issues grid: search on the item, the store and the source.
pub(crate) fn pending_grid(lang: &Lang, can_export: bool) -> Grid {
    let grid = Grid::new("inventory-pending")
        .title(&tr(lang, "pending.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::related(
                "item",
                &tr(lang, "stock.item"),
                "wl_inventory_items",
                "item_id",
                "name_en",
            )
            .frozen()
            .mobile()
            .searchable(),
        )
        .column(
            Column::related(
                "location",
                &tr(lang, "stock.store"),
                "wl_inventory_locations",
                "location_id",
                "name_en",
            )
            .mobile()
            .searchable(),
        )
        .column(
            Column::quantity("quantity_milli", &tr(lang, "pending.short"))
                .scale(3)
                .unit_from("unit")
                .mobile(),
        )
        .column(
            Column::related(
                "unit",
                &tr(lang, "stock.unit"),
                "wl_inventory_items",
                "item_id",
                "base_unit",
            )
            .hidden(),
        )
        .column(Column::custom("source", &tr(lang, "pending.source")))
        .column(Column::text("source_type", &tr(lang, "pending.source_type")).hidden().searchable())
        .column(Column::datetime("recorded_at", &tr(lang, "pending.recorded")))
        .column(Column::custom("state", &tr(lang, "fields.status")).mobile())
        .column(Column::custom("actions", &tr(lang, "stock.actions")))
        .sort_by("recorded_at")
        .cards_on_mobile()
        .empty_state(
            &tr(lang, "pending.empty"),
            Some(&tr(lang, "pending.empty_text")),
        );
    if can_export { grid.exports() } else { grid }
}

/// What the pending issues page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/pending.html")]
pub(crate) struct PendingPage {
    tx: Texts,
    rows: renox::grid::GridPage<PendingIssue>,
    approvals: Vec<ApprovalCard>,
    /// The pending issues the person may clear (a sheet each).
    clearing: Vec<PendingSheet>,
}

/// The pending issues page: the open ones at the locations where the user may
/// clear them, oldest first, each with the form that clears it (a reason and,
/// for a lot-tracked item, the lot the stock is entered in).
pub(crate) async fn pending_page(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let mut ids = Vec::new();
    for pending in pending_issue::open_of(&state.db).await? {
        if user.has_permission_in(permissions::STOCK_ADJUST, &pending.scope()) {
            ids.push(pending.id);
        }
    }
    let grid = pending_grid(lang, true);
    let query = PendingIssue::query().where_(PendingIssue::ID.is_in(ids_or_none(ids)));
    if let Some(file) = exported(&grid, &query, &request, true).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut cards = Vec::new();
    let mut clearing = Vec::new();
    let mut extra = std::collections::HashMap::new();
    for pending in page.items() {
        let (value, sheet) = pending_row(state, user, lang, pending, &mut cards).await?;
        extra.insert(pending.id, value);
        clearing.extend(sheet);
    }
    let page = page.extend(|pending| extra.get(&pending.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(PendingPage {
        tx: tx(lang),
        rows: page,
        approvals: cards,
        clearing,
    })
    .into_response())
}

/// What the grid shows beyond the columns of a pending issue, and the sheet
/// that clears it when the person may.
async fn pending_row(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    pending: &PendingIssue,
    cards: &mut Vec<ApprovalCard>,
) -> Result<(renox::serde_json::Value, Option<PendingSheet>)> {
    let item = Item::find_or_404(&state.db, pending.item_id).await?;
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
    let can_clear = requests.is_empty();
    let value = json!({
        "item": item_name(&item, lang),
        "source": label(state, &pending.source()).await,
        "waiting": !requests.is_empty(),
        "can_clear": can_clear,
    });
    let sheet = can_clear.then(|| PendingSheet {
        id: pending.id,
        lot_tracked: item.lot_tracked,
        lots,
    });
    Ok((value, sheet))
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
        PENDING_ISSUE_SUBJECT_TYPE, PURCHASE_ORDER_SUBJECT_TYPE, STOCK_HOLD_LINE_SUBJECT_TYPE,
        STOCK_HOLD_SUBJECT_TYPE, TRANSFER_SUBJECT_TYPE,
    };
    // The number alone: a page that names a lot (a recall's trace) never
    // names the product.
    register(LOT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Lot::find(db, id).await?.map(|lot| lot.number)) })
    });
    register(LOCATION_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Location::find(db, id).await?.map(|row| row.name_en)) })
    });
    register(STOCK_HOLD_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(StockHold::find(db, id).await?.and_then(|row| row.number)) })
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
