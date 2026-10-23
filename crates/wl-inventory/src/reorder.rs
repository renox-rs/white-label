//! Reorder suggestions (wl-inventory §3 *Reorder suggestion*, §5.9 step 3,
//! `STK.RUL.04`, issue #247). Every day at 07:00 Asia/Dubai [`run`] looks at
//! the items that have a reorder point and a preferred supplier row: where
//! the quantity at a location is at or below the reorder point it writes one
//! open [`ReorderSuggestion`] for the item, location and day (a second run
//! the same day updates it), and tells the purchasers.
//!
//! - The supplier is the preferred one. When that row is inactive for the
//!   item, or the supplier is blocked (`BlockGuard::check(supplier, start)`,
//!   `PTY.ACL.03`), the suggestion goes to the first active alternative, by
//!   rank, that passes the check, marked *alternative* with the reason the
//!   preferred one was skipped. With none left the suggestion has no
//!   supplier and the purchaser picks one. Nothing is ever suggested to a
//!   blocked supplier, and the other suppliers are listed with their lead
//!   time and last price, a blocked one as *on hold* only.
//! - A preferred row whose deactivation waits for approval is still active,
//!   so suggestions still go to it.
//! - The quantity fills up to the item's maximum (or par; the reorder point
//!   when there is none) and is raised to the supplier's minimum order
//!   quantity, the quantity before shown on the suggestion. Under the policy
//!   *reorder point + lead time* the suggestion carries the lead time of the
//!   supplier it goes to.
//! - A document kind the chosen row asks for that is not valid today is a
//!   warning on the suggestion; it is converted all the same.
//! - The purchaser can switch an open suggestion to another listed supplier
//!   with a reason ([`switch`]); that changes nothing on the item, is audited
//!   and is kept on the suggestion for the scorecard (who, when, and whether
//!   it was the preferred supplier that was left).
//!
//! - Back-ordered sales order quantity (`wl-sales`, setting *Back-orders
//!   allowed*, BL-SAL-17) not yet covered by an open purchase order is added
//!   to the quantity of the item's policy; above its reorder point (or with
//!   none) the suggestion is the back-ordered quantity alone, and an item
//!   without a preferred supplier is suggested without a supplier for it. A
//!   cancelled back-order drops out at the next run.
//!
//! Decisions: the quantity at a location is what the movements leave
//! (reservations and holds come with their own issues), the locations
//! looked at are those the item has ever moved through and those with a
//! back-order, and the suggestion mode is the default one (draft purchase
//! orders are not built).

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::item::{Item, ReorderPolicy};
use crate::item_supplier::{
    self, ItemSupplier, SupplierRole, audit, is_on_hold, last_price, refused, require, warnings_of,
};
use crate::movement::{self, StockMovement};
use crate::permissions;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use crate::texts::{english, said};

/// The daily scheduled task that runs [`run`].
pub const TASK: &str = "inventory:reorder-suggestions";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "07:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The notification type of the notice to the purchasers.
pub const NOTICE: &str = "inventory.reorder_suggested";

/// The role that is told.
pub const PROCUREMENT_ROLE: &str = "procurement";

/// The subject type of a reorder suggestion.
pub const SUBJECT_TYPE: &str = "inventory.reorder_suggestion";

/// The `dismiss` transition's name.
pub const DISMISS: &str = "dismiss";

/// The `convert` transition's name.
pub const CONVERT: &str = "convert";

/// Where a suggestion stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SuggestionStatus {
    /// Waiting for the purchaser.
    #[default]
    Open,
    /// Turned into a purchase order.
    Converted,
    /// Set aside.
    Dismissed,
}

/// A proposed purchase of an item for a location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_reorder_suggestions",
    state_machine,
    unique(item_id, location_id, run_on),
    index(status)
)]
pub struct ReorderSuggestion {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The location that is short.
    pub location_id: i64,
    /// The day of the run (Asia/Dubai).
    pub run_on: NaiveDate,
    /// The supplier it goes to; none when no supplier is usable.
    pub supplier_party_id: Option<i64>,
    /// `preferred` or `alternative`.
    pub supplier_role: Option<String>,
    /// Why the preferred supplier was skipped, or why a purchaser switched.
    pub reason: Option<String>,
    /// The lead time of the supplier, under the policy that uses one.
    pub lead_time_days: Option<i64>,
    /// The quantity to buy, in thousandths of the base unit.
    pub suggested_qty_milli: i64,
    /// The quantity before it was raised to the minimum order quantity.
    pub rounded_from_milli: Option<i64>,
    /// What the location had when the run looked.
    pub on_hand_milli: i64,
    /// The other suppliers shown, as JSON ([`Alternative`]).
    pub alternatives: String,
    /// The document kinds not valid for the supplier, as JSON.
    pub warnings: String,
    /// The lifecycle state; changed only by a transition.
    pub status: SuggestionStatus,
    /// Who switched the supplier.
    pub switched_by: Option<i64>,
    /// When it was switched.
    pub switched_at: Option<DateTime>,
    /// Why it was switched.
    pub switch_reason: Option<String>,
    /// The supplier it was switched from.
    pub switched_from_party_id: Option<i64>,
    /// Whether that supplier was the item's preferred one.
    pub switched_from_preferred: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ReorderSuggestion {
    /// The suggestion as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The other suppliers shown.
    pub fn alternatives(&self) -> Vec<Alternative> {
        renox::serde_json::from_str(&self.alternatives).unwrap_or_default()
    }

    /// The document kinds that are not valid for the supplier.
    pub fn warnings(&self) -> Vec<String> {
        renox::serde_json::from_str(&self.warnings).unwrap_or_default()
    }
}

impl StateMachine for ReorderSuggestion {
    type State = SuggestionStatus;
    const COLUMN: renox::db::Col<ReorderSuggestion, SuggestionStatus> = ReorderSuggestion::STATUS;

    fn machine() -> Machine<Self> {
        use SuggestionStatus::{Converted, Dismissed, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("info")
            .state(Converted)
            .tone("success")
            .terminal()
            .state(Dismissed)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(DISMISS)
                    .label("Dismiss")
                    .permission(permissions::PURCHASE_ORDERS_ISSUE)
                    .from([Open])
                    .to(Dismissed),
            )
            .transition(
                Transition::new(CONVERT)
                    .label("Convert")
                    .permission(permissions::PURCHASE_ORDERS_ISSUE)
                    .from([Open])
                    .to(Converted),
            )
    }
}

/// One supplier shown beside the one a suggestion goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    /// The supplier party.
    pub supplier_party_id: i64,
    /// `preferred` or `alternative`.
    pub role: String,
    /// Days from order to delivery.
    pub lead_time_days: i64,
    /// The last price per base unit, in fils, if there is one.
    pub last_price: Option<i64>,
    /// The currency of the last price.
    pub currency: Option<String>,
    /// The supplier is blocked: shown as *on hold*, with no reason.
    pub on_hold: bool,
    /// The document kinds not valid today for the supplier.
    pub warnings: Vec<String>,
    /// The supplier's current score; filled in once scores are kept.
    pub score: Option<i64>,
}

/// The notification type of the notice to the purchasers: staff,
/// transactional, by e-mail.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "inventory")
        .group("purchase_orders")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.reorder.subject")),
            english("wl_inventory.reorder.body"),
        )
}

/// Registers the notification type and the machine.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// Registers the daily [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
}

/// The supplier a suggestion goes to, and what is shown beside it.
#[derive(Debug, Clone)]
pub struct Choice {
    /// The row chosen, if any supplier is usable.
    pub row: Option<ItemSupplier>,
    /// Why the preferred supplier was skipped.
    pub reason: Option<String>,
    /// The other active suppliers.
    pub alternatives: Vec<Alternative>,
}

/// Describes row `row` for the list beside a suggestion.
async fn describe(state: &AppState, row: &ItemSupplier) -> Result<Alternative> {
    let price = last_price(&state.db, row.item_id, row.supplier_party_id).await?;
    let on_hold = is_on_hold(state, row.supplier_party_id).await?;
    Ok(Alternative {
        supplier_party_id: row.supplier_party_id,
        role: row.role.as_str().to_owned(),
        lead_time_days: row.lead_time_days,
        last_price: price.as_ref().map(|price| price.unit_price),
        currency: price.map(|price| price.currency),
        on_hold,
        warnings: warnings_of(state, row).await?,
        score: None,
    })
}

/// Picks the supplier for an item with `rows` (preferred first, then by
/// rank): the preferred one, or the first active alternative that is not on
/// hold.
///
/// # Errors
///
/// A database error.
pub async fn choose(state: &AppState, rows: &[ItemSupplier]) -> Result<Choice> {
    let mut on_hold = Vec::new();
    for row in rows {
        on_hold.push(row.active && is_on_hold(state, row.supplier_party_id).await?);
    }
    let mut chosen: Option<usize> = None;
    let mut reason = None;
    if let Some(at) = rows
        .iter()
        .position(|row| row.role == SupplierRole::Preferred)
    {
        if rows[at].active && !on_hold[at] {
            chosen = Some(at);
        } else if rows[at].active {
            reason = Some(said("wl_inventory.reorder.preferred_on_hold", &[]));
        } else {
            reason = Some(said("wl_inventory.reorder.preferred_inactive", &[]));
        }
    }
    if chosen.is_none() {
        chosen = rows.iter().enumerate().position(|(index, row)| {
            row.role == SupplierRole::Alternative && row.active && !on_hold[index]
        });
    }
    let mut alternatives = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.active && Some(index) != chosen {
            alternatives.push(describe(state, row).await?);
        }
    }
    Ok(Choice {
        row: chosen.map(|index| rows[index].clone()),
        reason,
        alternatives,
    })
}

/// The quantity to buy when `on_hand_milli` is at or below the reorder
/// point: up to the maximum (or par), or to the reorder point when the item
/// has none. 0 when nothing is to be bought.
pub fn wanted_milli(item: &Item, on_hand_milli: i64) -> i64 {
    let target = item.reorder_max_milli.max(item.reorder_point_milli);
    (target - on_hand_milli).max(0)
}

/// `wanted_milli` raised to the minimum order quantity of `row`: the
/// quantity, and what it was before when it was raised.
pub fn rounded_milli(wanted_milli: i64, row: Option<&ItemSupplier>) -> (i64, Option<i64>) {
    match row {
        Some(row) if wanted_milli < row.min_order_qty_milli => {
            (row.min_order_qty_milli, Some(wanted_milli))
        }
        _ => (wanted_milli, None),
    }
}

/// The lead time a suggestion carries: that of the supplier it goes to, for
/// the policy *reorder point + lead time*.
pub fn lead_time_for(item: &Item, row: Option<&ItemSupplier>) -> Option<i64> {
    match (item.reorder_policy, row) {
        (ReorderPolicy::ReorderPointLeadTime, Some(row)) => Some(row.lead_time_days),
        _ => None,
    }
}

fn json_of<T: Serialize>(value: &T) -> String {
    renox::serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned())
}

/// The back-ordered quantity of sales orders for `item` at `location_id` not
/// yet covered by an open purchase order, in thousandths, from the `demand`
/// of the sales module (BL-SAL-17). `cover_milli` is what the open purchase
/// orders of the item still have outstanding; it is used up location by
/// location, in the order of the ids, so a quantity is covered once.
pub fn uncovered_milli(demand_milli: i64, cover_milli: &mut i64) -> i64 {
    let covered = demand_milli.min(*cover_milli).max(0);
    *cover_milli -= covered;
    demand_milli - covered
}

/// What the open purchase orders of `item_id` still have to come, in
/// thousandths: ordered less accepted, over the lines of orders that are
/// issued or partly received.
///
/// # Errors
///
/// A database error.
pub async fn on_order_milli(db: &Db, item_id: i64) -> Result<i64> {
    let lines = PurchaseOrderLine::query()
        .where_(PurchaseOrderLine::ITEM_ID.eq(item_id))
        .get(db)
        .await?;
    let mut open = 0;
    for line in lines {
        let Some(order) = PurchaseOrder::find(db, line.purchase_order_id).await? else {
            continue;
        };
        if matches!(
            order.status,
            PurchaseOrderStatus::Issued | PurchaseOrderStatus::PartiallyReceived
        ) {
            open += line.outstanding_milli();
        }
    }
    Ok(open)
}

/// Runs `STK.RUL.04` for today: one open suggestion per item and location
/// that is at or below its reorder point, or that sales orders have
/// back-ordered. The suggested quantity is what the item's policy asks for
/// (nothing above the reorder point) plus the back-ordered quantity not yet
/// covered by an open purchase order (BL-SAL-17); an item with a back-order
/// and no preferred supplier is suggested without a supplier. Returns how
/// many were written (made or updated).
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn run(state: &AppState) -> Result<usize> {
    let day = today();
    let mut items = Item::query().order_by(Item::ID).get(&state.db).await?;
    // The open back-orders of sales orders, per item code and location.
    let mut back_ordered: BTreeMap<String, BTreeMap<i64, i64>> = BTreeMap::new();
    for open in wl_kernel::backorders::back_orders(state)
        .open(state)
        .await?
    {
        if let Some(location) = open.location {
            *back_ordered
                .entry(open.item_code)
                .or_default()
                .entry(location.subject_id)
                .or_insert(0) += open.quantity_milli;
        }
    }
    items.retain(|item| item.reorder_point_milli > 0 || back_ordered.contains_key(&item.code));
    let mut written = 0;
    let mut made: Vec<ReorderSuggestion> = Vec::new();
    let mut touched: BTreeSet<(i64, i64)> = BTreeSet::new();
    for item in items {
        let rows = item_supplier::rows_of(&state.db, item.id).await?;
        let has_preferred = rows.iter().any(|row| row.role == SupplierRole::Preferred);
        let demand = back_ordered.get(&item.code).cloned().unwrap_or_default();
        if !has_preferred && demand.is_empty() {
            continue;
        }
        let moves = StockMovement::query()
            .where_(StockMovement::ITEM_ID.eq(item.id))
            .get(&state.db)
            .await?;
        let mut locations = BTreeSet::new();
        for moved in &moves {
            locations.extend(moved.to_location_id);
            locations.extend(moved.from_location_id);
        }
        locations.extend(demand.keys().copied());
        let mut on_order = if demand.is_empty() {
            0
        } else {
            on_order_milli(&state.db, item.id).await?
        };
        for location_id in locations {
            let on_hand = movement::on_hand(&state.db, item.id, location_id).await?;
            // The policy asks for stock at or below the reorder point; the
            // back-ordered quantity is added on top, whatever the stock.
            let policy = if has_preferred && on_hand <= item.reorder_point_milli {
                wanted_milli(&item, on_hand)
            } else {
                0
            };
            let back = uncovered_milli(
                demand.get(&location_id).copied().unwrap_or(0),
                &mut on_order,
            );
            let wanted = policy + back;
            if wanted == 0 {
                continue;
            }
            touched.insert((item.id, location_id));
            let choice = choose(state, &rows).await?;
            let (quantity, from) = rounded_milli(wanted, choice.row.as_ref());
            let warnings = match &choice.row {
                Some(row) => warnings_of(state, row).await?,
                None => Vec::new(),
            };
            let supplier = choice.row.as_ref().map(|row| row.supplier_party_id);
            let role = choice.row.as_ref().map(|row| row.role.as_str().to_owned());
            let lead = lead_time_for(&item, choice.row.as_ref());
            let alternatives = json_of(&choice.alternatives);
            let warnings = json_of(&warnings);
            let existing = ReorderSuggestion::query()
                .where_(ReorderSuggestion::ITEM_ID.eq(item.id))
                .where_(ReorderSuggestion::LOCATION_ID.eq(location_id))
                .where_(ReorderSuggestion::RUN_ON.eq(day))
                .first(&state.db)
                .await?;
            match existing {
                Some(found) if found.status != SuggestionStatus::Open => {}
                // A purchaser's choice is kept by a second run.
                Some(found) if found.switched_by.is_some() => {}
                Some(found) => {
                    let now = renox::db::now();
                    ReorderSuggestion::query()
                        .where_(ReorderSuggestion::ID.eq(found.id))
                        .update(
                            &state.db,
                            &[
                                ("supplier_party_id", &supplier),
                                ("supplier_role", &role),
                                ("reason", &choice.reason),
                                ("lead_time_days", &lead),
                                ("suggested_qty_milli", &quantity),
                                ("rounded_from_milli", &from),
                                ("on_hand_milli", &on_hand),
                                ("alternatives", &alternatives),
                                ("warnings", &warnings),
                                ("updated_at", &now),
                            ],
                        )
                        .await?;
                    written += 1;
                }
                None => {
                    let mut suggestion = ReorderSuggestion {
                        item_id: item.id,
                        location_id,
                        run_on: day,
                        supplier_party_id: supplier,
                        supplier_role: role,
                        reason: choice.reason.clone(),
                        lead_time_days: lead,
                        suggested_qty_milli: quantity,
                        rounded_from_milli: from,
                        on_hand_milli: on_hand,
                        alternatives,
                        warnings,
                        status: SuggestionStatus::Open,
                        ..ReorderSuggestion::default()
                    };
                    suggestion.save(&state.db).await?;
                    made.push(suggestion);
                    written += 1;
                }
            }
        }
    }
    drop_cancelled_back_orders(state, day, &touched).await?;
    tell_purchasers(state, &made).await?;
    Ok(written)
}

/// An item without a reorder point is suggested only for its back-orders: a
/// suggestion of today that this run did not touch lost its back-order (it
/// was cancelled, served or covered by a purchase order since the earlier run
/// today) and is dropped, unless a purchaser already switched its supplier.
async fn drop_cancelled_back_orders(
    state: &AppState,
    day: NaiveDate,
    touched: &BTreeSet<(i64, i64)>,
) -> Result {
    let open = ReorderSuggestion::query()
        .where_(ReorderSuggestion::RUN_ON.eq(day))
        .where_(ReorderSuggestion::STATUS.eq(SuggestionStatus::Open))
        .where_(ReorderSuggestion::SWITCHED_BY.is_null())
        .get(&state.db)
        .await?;
    for suggestion in open {
        if touched.contains(&(suggestion.item_id, suggestion.location_id)) {
            continue;
        }
        let Some(item) = Item::find(&state.db, suggestion.item_id).await? else {
            continue;
        };
        if item.reorder_point_milli == 0 {
            ReorderSuggestion::query()
                .where_(ReorderSuggestion::ID.eq(suggestion.id))
                .delete(&state.db)
                .await?;
        }
    }
    Ok(())
}

/// Tells the holders of the procurement role about the suggestions made.
async fn tell_purchasers(state: &AppState, made: &[ReorderSuggestion]) -> Result<()> {
    let Some(first) = made.first() else {
        return Ok(());
    };
    let people = crate::chase::holders(&state.db, PROCUREMENT_ROLE).await?;
    if people.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(NOTICE, first.subject())
        .subject_label("reorder suggestions")
        .data(json!({ "count": made.len().to_string() }));
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Switches the open suggestion `id` to `supplier`, one of the suppliers
/// listed for the item, with a `reason` (`purchase_orders.issue`). Nothing
/// changes on the item; the switch is audited and kept on the suggestion.
///
/// # Errors
///
/// `Forbidden` without the permission; 422 without a reason, for a
/// suggestion that is not open, a supplier that is not listed and active
/// for the item, or one that is on hold.
pub async fn switch(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    supplier: i64,
    reason: &str,
) -> Result<ReorderSuggestion> {
    require(state, user, permissions::PURCHASE_ORDERS_ISSUE).await?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(refused(said("wl_inventory.reorder.reason_needed", &[])));
    }
    let suggestion = ReorderSuggestion::find_or_404(&state.db, id).await?;
    if suggestion.status != SuggestionStatus::Open {
        return Err(refused(said("wl_inventory.reorder.not_open", &[])));
    }
    let rows = item_supplier::rows_of(&state.db, suggestion.item_id).await?;
    let Some(row) = rows
        .iter()
        .find(|row| row.active && row.supplier_party_id == supplier)
    else {
        return Err(refused(said("wl_inventory.reorder.not_listed", &[])));
    };
    if suggestion.supplier_party_id == Some(supplier) {
        return Err(refused(said("wl_inventory.reorder.same", &[])));
    }
    if is_on_hold(state, supplier).await? {
        return Err(refused(said("wl_inventory.reorder.on_hold", &[])));
    }
    let item = Item::find_or_404(&state.db, suggestion.item_id).await?;
    let from_preferred = rows.iter().any(|other| {
        other.role == SupplierRole::Preferred
            && Some(other.supplier_party_id) == suggestion.supplier_party_id
    });
    let before = suggestion
        .rounded_from_milli
        .unwrap_or(suggestion.suggested_qty_milli);
    let (quantity, from) = rounded_milli(before, Some(row));
    let mut alternatives = Vec::new();
    for other in rows
        .iter()
        .filter(|other| other.active && other.supplier_party_id != supplier)
    {
        alternatives.push(describe(state, other).await?);
    }
    let warnings = warnings_of(state, row).await?;
    let role = Some(row.role.as_str().to_owned());
    let supplier_id = Some(supplier);
    let lead = lead_time_for(&item, Some(row));
    let note = Some(said("wl_inventory.reorder.switched", &[]));
    let by = Some(user.id);
    let at = Some(renox::db::now());
    let left = suggestion.supplier_party_id;
    let reason_text = Some(reason.to_owned());
    let alternatives_json = json_of(&alternatives);
    let warnings_json = json_of(&warnings);
    let now = renox::db::now();
    ReorderSuggestion::query()
        .where_(ReorderSuggestion::ID.eq(suggestion.id))
        .update(
            &state.db,
            &[
                ("supplier_party_id", &supplier_id),
                ("supplier_role", &role),
                ("reason", &note),
                ("lead_time_days", &lead),
                ("suggested_qty_milli", &quantity),
                ("rounded_from_milli", &from),
                ("alternatives", &alternatives_json),
                ("warnings", &warnings_json),
                ("switched_by", &by),
                ("switched_at", &at),
                ("switch_reason", &reason_text),
                ("switched_from_party_id", &left),
                ("switched_from_preferred", &from_preferred),
                ("updated_at", &now),
            ],
        )
        .await?;
    audit(
        state,
        Some(user.id),
        "inventory.reorder_switched",
        ReorderSuggestion::TABLE,
        suggestion.id,
        json!({
            "item": suggestion.item_id,
            "from": left,
            "to": supplier,
            "reason": reason,
            "from_preferred": from_preferred,
        }),
    )
    .await?;
    ReorderSuggestion::find_or_404(&state.db, suggestion.id).await
}

/// How many suggestions each user switched away from the preferred supplier
/// in the calendar month of `month_start` (Asia/Dubai): the scorecard's
/// purchaser view.
///
/// # Errors
///
/// A database error.
pub async fn switches_away_from_preferred(
    db: &Db,
    month_start: NaiveDate,
) -> Result<Vec<(i64, usize)>> {
    let rows = ReorderSuggestion::query()
        .where_(ReorderSuggestion::SWITCHED_FROM_PREFERRED.eq(true))
        .get(db)
        .await?;
    let mut counts: Vec<(i64, usize)> = Vec::new();
    for row in rows {
        let (Some(by), Some(at)) = (row.switched_by, row.switched_at) else {
            continue;
        };
        let day = wl_kernel::documents::dubai_date(at);
        if (day.year(), day.month()) != (month_start.year(), month_start.month()) {
            continue;
        }
        match counts.iter_mut().find(|(user, _)| *user == by) {
            Some((_, count)) => *count += 1,
            None => counts.push((by, 1)),
        }
    }
    counts.sort_unstable();
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(point: i64, max: i64) -> Item {
        Item::new("X", "X", "kg").reordered(point, max)
    }

    #[test]
    fn the_wanted_quantity_fills_up_to_the_maximum_or_the_point() {
        assert_eq!(wanted_milli(&item(20_000, 100_000), 15_000), 85_000);
        assert_eq!(wanted_milli(&item(20_000, 0), 15_000), 5_000);
        assert_eq!(wanted_milli(&item(20_000, 100_000), 120_000), 0);
    }

    #[test]
    fn the_quantity_is_raised_to_the_minimum_order_and_shows_what_it_was() {
        let row = ItemSupplier {
            min_order_qty_milli: 500_000,
            ..ItemSupplier::default()
        };
        assert_eq!(rounded_milli(130_000, Some(&row)), (500_000, Some(130_000)));
        assert_eq!(rounded_milli(600_000, Some(&row)), (600_000, None));
        assert_eq!(rounded_milli(130_000, None), (130_000, None));
    }

    #[test]
    fn only_the_lead_time_policy_takes_the_lead_time() {
        let row = ItemSupplier {
            lead_time_days: 10,
            ..ItemSupplier::default()
        };
        let plain = item(1, 2);
        assert_eq!(lead_time_for(&plain, Some(&row)), None);
        let timed = item(1, 2).with_policy(ReorderPolicy::ReorderPointLeadTime);
        assert_eq!(lead_time_for(&timed, Some(&row)), Some(10));
        assert_eq!(lead_time_for(&timed, None), None);
    }

    #[test]
    fn a_suggestion_starts_open_and_can_be_dismissed_or_converted() {
        let machine = ReorderSuggestion::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        assert_eq!(machine.find(DISMISS).unwrap().to_state(), Some("dismissed"));
        assert_eq!(machine.find(CONVERT).unwrap().to_state(), Some("converted"));
    }

    #[test]
    fn stk_rul_04_an_open_purchase_order_covers_a_back_order_once() {
        // 3 back-ordered at one location, 1 on order: 2 are uncovered.
        let mut cover = 1_000;
        assert_eq!(uncovered_milli(3_000, &mut cover), 2_000);
        assert_eq!(cover, 0);
        // The cover is used up across locations, never twice.
        let mut cover = 4_000;
        assert_eq!(uncovered_milli(3_000, &mut cover), 0);
        assert_eq!(uncovered_milli(3_000, &mut cover), 2_000);
        // Nothing on order: the whole back-order is demand.
        let mut none = 0;
        assert_eq!(uncovered_milli(2_000, &mut none), 2_000);
    }
}
