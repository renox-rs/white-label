//! The actions on the suppliers of an item and on reorder suggestions
//! (wl-inventory §3, §5.9, issue #247):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /items/{id}/suppliers` | `wl_inventory.item_suppliers` | `items.view`, `item_suppliers.manage` or `purchase_orders.issue` |
//! | `POST /items/{id}/suppliers/add` | `wl_inventory.item_supplier_add` | `item_suppliers.manage` |
//! | `POST /item-suppliers/{id}/deactivate` | `wl_inventory.item_supplier_deactivate` | `item_suppliers.manage` |
//! | `GET /reorder-suggestions/{id}` | `wl_inventory.reorder_show` | as the suppliers |
//! | `POST /reorder-suggestions/{id}/switch` | `wl_inventory.reorder_switch` | `purchase_orders.issue` |
//! | `GET /suppliers/{party}/score` | `wl_inventory.supplier_score` | `supplier_scores.view` |
//! | `POST /supplier-scores/{id}/notes` | `wl_inventory.supplier_score_note` | `purchase_orders.issue` |
//! | `POST /supplier-reviews/{id}/close` | `wl_inventory.supplier_review_close` | `purchase_orders.issue` |
//!
//! The last price, and the values behind a supplier score, are part of the
//! answers only for a holder of `stock.cost.view`; a blocked supplier shows
//! as `on_hold` without a reason.

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::Serialize;

use crate::item_supplier::{
    self, DeactivateOutcome, LastPrice, NewItemSupplier, SupplierRole, holds,
    is_on_hold, last_price, refused, rows_of, warnings_of,
};
use crate::pages::parse_quantity;
use crate::permissions;
use crate::reorder::{Alternative, ReorderSuggestion};
use crate::supplier_score::{Figures, SupplierScore};
use crate::texts;

/// The routes.
pub(crate) fn routes() -> Routes {
    Routes::new()
        .get("/items/{id}/suppliers", list_suppliers)
        .name("wl_inventory.item_suppliers")
        .post("/items/{id}/suppliers/add", add_supplier)
        .name("wl_inventory.item_supplier_add")
        .post("/item-suppliers/{id}/deactivate", deactivate_supplier)
        .name("wl_inventory.item_supplier_deactivate")
        .get("/reorder-suggestions/{id}", show_suggestion)
        .name("wl_inventory.reorder_show")
        .post("/reorder-suggestions/{id}/switch", switch_suggestion)
        .name("wl_inventory.reorder_switch")
        .get("/suppliers/{party}/score", show_score)
        .name("wl_inventory.supplier_score")
        .post("/supplier-scores/{id}/notes", add_score_note)
        .name("wl_inventory.supplier_score_note")
        .post("/supplier-reviews/{id}/close", close_review)
        .name("wl_inventory.supplier_review_close")
        .require_auth()
}

fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_inventory.{key}"))
}

/// An answer in JSON.
fn json_response<T: Serialize>(value: &T) -> Result<Response> {
    let body = renox::serde_json::to_string(value)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// Whether `user` may read the suppliers of items.
async fn may_read(state: &AppState, user: &AuthUser) -> Result<bool> {
    for permission in [
        permissions::ITEMS_VIEW,
        permissions::ITEM_SUPPLIERS_MANAGE,
        permissions::PURCHASE_ORDERS_ISSUE,
    ] {
        if holds(state, user, permission).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One supplier of an item as the back office reads it.
#[derive(Debug, Serialize)]
struct SupplierView {
    id: i64,
    supplier_party_id: i64,
    role: &'static str,
    rank: i64,
    active: bool,
    supplier_item_code: Option<String>,
    purchase_unit: Option<String>,
    lead_time_days: i64,
    min_order_qty_milli: i64,
    on_hold: bool,
    watched: bool,
    warnings: Vec<String>,
    last_price: Option<LastPrice>,
}

/// `GET /items/{id}/suppliers`: the preferred supplier first, then the
/// alternatives by rank.
async fn list_suppliers(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !may_read(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let costs = holds(&state, &user, permissions::STOCK_COST_VIEW).await?;
    let mut views = Vec::new();
    for row in rows_of(&state.db, id).await? {
        let price = if costs {
            last_price(&state.db, id, row.supplier_party_id).await?
        } else {
            None
        };
        views.push(SupplierView {
            id: row.id,
            supplier_party_id: row.supplier_party_id,
            role: row.role.as_str(),
            rank: row.rank,
            active: row.active,
            supplier_item_code: row.supplier_item_code.clone(),
            purchase_unit: row.purchase_unit.clone(),
            lead_time_days: row.lead_time_days,
            min_order_qty_milli: row.min_order_qty_milli,
            on_hold: row.active && is_on_hold(&state, row.supplier_party_id).await?,
            watched: crate::supplier_review::is_watched(&state, row.supplier_party_id).await?,
            warnings: warnings_of(&state, &row).await?,
            last_price: price,
        });
    }
    json_response(&views)
}

/// The supplier a form describes: `supplier`, `role`, optionally `rank`,
/// `code`, `unit`, `lead_days`, `min_qty` (base units), `certificates`
/// (comma separated) and `notes`.
fn supplier_of(
    item_id: i64,
    form: &BTreeMap<String, String>,
) -> std::result::Result<NewItemSupplier, &'static str> {
    let field = |name: &str| form.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
    let supplier: i64 = field("supplier")
        .and_then(|v| v.parse().ok())
        .ok_or("supplier.bad_input")?;
    let role = field("role")
        .and_then(SupplierRole::parse)
        .ok_or("supplier.bad_input")?;
    let rank = match field("rank") {
        Some(rank) => Some(rank.parse::<i64>().map_err(|_| "supplier.bad_input")?),
        None => None,
    };
    let lead_time_days = match field("lead_days") {
        Some(days) => days.parse::<i64>().map_err(|_| "supplier.bad_input")?,
        None => 0,
    };
    let min_order_qty_milli = match field("min_qty") {
        Some(quantity) => parse_quantity(quantity).ok_or("supplier.bad_input")?,
        None => 0,
    };
    Ok(NewItemSupplier {
        item_id,
        supplier,
        role,
        rank,
        supplier_item_code: field("code").map(str::to_owned),
        purchase_unit: field("unit").map(str::to_owned),
        lead_time_days,
        min_order_qty_milli,
        certificate_kinds: field("certificates")
            .map(|kinds| {
                kinds
                    .split(',')
                    .map(str::trim)
                    .filter(|kind| !kind.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        notes: field("notes").map(str::to_owned),
    })
}

/// `POST /items/{id}/suppliers/add`.
async fn add_supplier(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let new = supplier_of(id, &form).map_err(|key| refused(text(&lang, key)))?;
    item_supplier::add(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "supplier.added")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /item-suppliers/{id}/deactivate`.
async fn deactivate_supplier(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let key = match item_supplier::deactivate(&state, &user, id).await? {
        DeactivateOutcome::Deactivated(_) => "supplier.deactivated",
        DeactivateOutcome::Requested(_) => "supplier.requested",
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// A suggestion as the back office reads it.
#[derive(Debug, Serialize)]
struct SuggestionView {
    id: i64,
    item_id: i64,
    location_id: i64,
    supplier_party_id: Option<i64>,
    supplier_role: Option<String>,
    reason: Option<String>,
    lead_time_days: Option<i64>,
    suggested_qty_milli: i64,
    rounded_from_milli: Option<i64>,
    alternatives: Vec<Alternative>,
    warnings: Vec<String>,
    status: &'static str,
}

/// `GET /reorder-suggestions/{id}`: without `stock.cost.view` the last
/// prices of the suppliers are left out.
async fn show_suggestion(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !may_read(&state, &user).await? {
        return Err(Error::Forbidden);
    }
    let suggestion = ReorderSuggestion::find_or_404(&state.db, id).await?;
    let costs = holds(&state, &user, permissions::STOCK_COST_VIEW).await?;
    let mut alternatives = suggestion.alternatives();
    if !costs {
        for alternative in &mut alternatives {
            alternative.last_price = None;
            alternative.currency = None;
        }
    }
    json_response(&SuggestionView {
        id: suggestion.id,
        item_id: suggestion.item_id,
        location_id: suggestion.location_id,
        supplier_party_id: suggestion.supplier_party_id,
        supplier_role: suggestion.supplier_role.clone(),
        reason: suggestion.reason.clone(),
        lead_time_days: suggestion.lead_time_days,
        suggested_qty_milli: suggestion.suggested_qty_milli,
        rounded_from_milli: suggestion.rounded_from_milli,
        alternatives,
        warnings: suggestion.warnings(),
        status: match suggestion.status {
            crate::reorder::SuggestionStatus::Open => "open",
            crate::reorder::SuggestionStatus::Converted => "converted",
            crate::reorder::SuggestionStatus::Dismissed => "dismissed",
        },
    })
}

/// `POST /reorder-suggestions/{id}/switch`: `supplier` and `reason`.
async fn switch_suggestion(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let supplier: i64 = form
        .get("supplier")
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| refused(text(&lang, "supplier.bad_input")))?;
    let reason = form.get("reason").map(String::as_str).unwrap_or_default();
    crate::reorder::switch(&state, &user, id, supplier, reason).await?;
    Ok((
        Toast::success(text(&lang, "reorder.switched_done")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The numbers of a window as the back office reads them.
#[derive(Debug, Serialize)]
struct ScoreView {
    id: Option<i64>,
    snapshot_date: Option<renox::chrono::NaiveDate>,
    from: renox::chrono::NaiveDate,
    to: renox::chrono::NaiveDate,
    receipts: i64,
    on_time: i64,
    on_time_bp: Option<i64>,
    rejected_bp: Option<i64>,
    incidents: i64,
    debit_notes: i64,
    price_variance_bp: Option<i64>,
    score: Option<i64>,
    band: &'static str,
    received_value: Option<i64>,
    rejected_value: Option<i64>,
    debit_note_value: Option<i64>,
    notes: Vec<String>,
}

fn score_view(
    figures: &Figures,
    id: Option<i64>,
    snapshot_date: Option<renox::chrono::NaiveDate>,
    costs: bool,
    notes: Vec<String>,
) -> ScoreView {
    ScoreView {
        id,
        snapshot_date,
        from: figures.from,
        to: figures.to,
        receipts: figures.receipts,
        on_time: figures.on_time,
        on_time_bp: figures.on_time_bp,
        rejected_bp: figures.rejected_bp,
        incidents: figures.incidents,
        debit_notes: figures.debit_notes,
        price_variance_bp: figures.price_variance_bp,
        score: figures.score,
        band: figures.band.as_str(),
        received_value: costs.then_some(figures.received_value),
        rejected_value: costs.then_some(figures.rejected_value),
        debit_note_value: costs.then_some(figures.debit_note_value),
        notes,
    }
}

/// `GET /suppliers/{party}/score`: the live view and the snapshots, newest
/// first. The values need `stock.cost.view`.
async fn show_score(
    State(state): State<AppState>,
    user: AuthUser,
    Path(party): Path<i64>,
) -> Result<Response> {
    item_supplier::require(&state, &user, permissions::SUPPLIER_SCORES_VIEW).await?;
    let costs = holds(&state, &user, permissions::STOCK_COST_VIEW).await?;
    let live = crate::supplier_score::live(&state, party).await?;
    let mut snapshots = SupplierScore::query()
        .where_(SupplierScore::SUPPLIER_PARTY_ID.eq(party))
        .get(&state.db)
        .await?;
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.snapshot_date));
    let mut kept = Vec::new();
    for snapshot in &snapshots {
        let notes = crate::supplier_score::notes_of(&state.db, snapshot.id)
            .await?
            .into_iter()
            .map(|note| note.note)
            .collect();
        kept.push(score_view(
            &snapshot.figures(),
            Some(snapshot.id),
            Some(snapshot.snapshot_date),
            costs,
            notes,
        ));
    }
    let review = crate::supplier_review::open_review(&state.db, party).await?;
    json_response(&renox::serde_json::json!({
        "live": score_view(&live, None, None, costs, Vec::new()),
        "snapshots": kept,
        "watched": crate::supplier_review::is_watched(&state, party).await?,
        "open_review": review.map(|review| review.id),
    }))
}

/// `POST /supplier-scores/{id}/notes`: `note`.
async fn add_score_note(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let note = form.get("note").map(String::as_str).unwrap_or_default();
    crate::supplier_score::add_note(&state, &user, id, note).await?;
    Ok((
        Toast::success(text(&lang, "score.noted")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /supplier-reviews/{id}/close`: `decision` (`keep`, `watch`,
/// `change_preferred` or `request_block`) and, for `change_preferred`,
/// `items` (item ids, comma separated).
async fn close_review(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    use crate::supplier_review::Decision;
    let decision = match form.get("decision").map(|v| v.trim()) {
        Some("keep") => Decision::Keep,
        Some("watch") => Decision::Watch,
        Some("request_block") => Decision::RequestBlock,
        Some("change_preferred") => Decision::ChangePreferred {
            items: form
                .get("items")
                .map(String::as_str)
                .unwrap_or_default()
                .split(',')
                .filter_map(|item| item.trim().parse().ok())
                .collect(),
        },
        _ => return Err(refused(text(&lang, "review.bad_decision"))),
    };
    match crate::supplier_review::close(&state, &user, id, decision).await {
        Ok(_) => Ok((
            Toast::success(text(&lang, "review.closed")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(renox::state_machine::TransitionError::Guard(message)) => Err(refused(message)),
        Err(other) => Err(other.into_error()),
    }
}
