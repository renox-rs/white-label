//! The suppliers of an item (wl-inventory §3 *Item supplier*, §5.9 steps 1
//! and 3, `STK.SUP.01`, issue #247): one `preferred` supplier and ranked
//! `alternative`s per item, each with the supplier's item code, purchase
//! unit, lead time, minimum order quantity and the document kinds (a Halal
//! certificate) it should hold for the item.
//!
//! - One row per item and supplier; a second row for the same pair is
//!   refused. At most one row is `preferred`.
//! - The first preferred supplier of an item that never had one is set
//!   directly by `item_suppliers.manage`; any later preferred supplier, and
//!   deactivating the preferred row, is a [`PreferredSupplierRequest`]
//!   (`STK.ACL.07`, decided by BL-STK-31): the row stays as it is and the
//!   suggestions still go to it until the request is approved.
//! - Alternatives are ranked 1..n, renumbered on every save; a deactivated
//!   row keeps its rank and is skipped. Adding and deactivating an
//!   alternative needs no approval and is audited.
//! - The last price is derived, never stored: the unit price of the latest
//!   posted receipt line from the supplier for the item, else of the latest
//!   issued order line, in the document's currency ([`last_price`]).
//! - A supplier that is blocked (`BlockGuard::check(supplier, start)`) shows
//!   as *on hold*, without the reason. A document kind of the row that is not
//!   valid today for the supplier is a warning that blocks nothing.
//! - When two supplier parties are merged (`PartiesMerged`) their rows move
//!   to the survivor; where both are on one item the stronger row stays
//!   (preferred, else the better rank).
//!
//! Decisions: quantities and prices are per base unit, so the purchase unit
//! is kept as a label and the minimum order quantity is in base units
//! (unit conversions come with the catalogue). A party that is archived
//! leaves its rows alone here, as the party module emits no event for it.

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};
use wl_kernel::block_guard::{ActionKind, guard};
use wl_kernel::documents;
use wl_kernel::{PartiesMerged, PartyId};

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use crate::item::Item;
use crate::permissions;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use crate::texts::said;

/// Whether the supplier is the item's preferred one or an alternative.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SupplierRole {
    /// One of the ranked alternatives.
    #[default]
    Alternative,
    /// The supplier suggestions go to.
    Preferred,
}

impl SupplierRole {
    /// The role called `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "preferred" => Some(SupplierRole::Preferred),
            "alternative" => Some(SupplierRole::Alternative),
            _ => None,
        }
    }
}

/// A supplier that can deliver an item.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_item_suppliers",
    unique(item_id, supplier_party_id)
)]
pub struct ItemSupplier {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The supplier party.
    pub supplier_party_id: i64,
    /// Preferred or alternative.
    pub role: SupplierRole,
    /// Among the alternatives, from 1; 0 for the preferred supplier.
    pub rank: i64,
    /// The supplier's own code for the item.
    pub supplier_item_code: Option<String>,
    /// The unit the supplier sells in, as a label.
    pub purchase_unit: Option<String>,
    /// Days from order to delivery.
    pub lead_time_days: i64,
    /// The smallest order, in thousandths of the base unit; 0 for none.
    pub min_order_qty_milli: i64,
    /// Whether the row is in use.
    pub active: bool,
    /// The document kinds the supplier should hold for the item, comma
    /// separated.
    pub certificate_kinds: String,
    /// A free note.
    pub notes: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ItemSupplier {
    /// The supplier as a [`PartyId`].
    pub fn supplier(&self) -> PartyId {
        PartyId(self.supplier_party_id)
    }

    /// The document kinds the row asks for.
    pub fn kinds(&self) -> Vec<String> {
        self.certificate_kinds
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// Where a request to change the preferred supplier stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RequestStatus {
    /// Waiting for a second person (BL-STK-31).
    #[default]
    Pending,
    /// Approved.
    Approved,
    /// Rejected.
    Rejected,
}

/// A request to change who is the preferred supplier of an item
/// (`STK.ACL.07`): decided by a holder of `item_suppliers.approve` who is not
/// the requester (BL-STK-31).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_preferred_supplier_requests", index(item_id))]
pub struct PreferredSupplierRequest {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The current preferred supplier.
    pub from_supplier_party_id: i64,
    /// The supplier to make preferred; none when the row is only deactivated.
    pub to_supplier_party_id: Option<i64>,
    /// `deactivate` or `replace`.
    pub kind: String,
    /// Where it stands.
    pub status: RequestStatus,
    /// Who asked.
    pub requested_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `Forbidden` unless `user` holds `permission` (globally).
pub(crate) async fn require(state: &AppState, user: &AuthUser, permission: &str) -> Result<()> {
    let holders = users_with_permission_in(&state.db, permission, &Scope::global()).await?;
    if holders.iter().any(|holder| holder.id == user.id) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Whether `user` holds `permission` (globally).
pub(crate) async fn holds(state: &AppState, user: &AuthUser, permission: &str) -> Result<bool> {
    Ok(
        users_with_permission_in(&state.db, permission, &Scope::global())
            .await?
            .iter()
            .any(|holder| holder.id == user.id),
    )
}

/// Writes an audit entry about record `id` of `table`.
pub(crate) async fn audit(
    state: &AppState,
    user: Option<i64>,
    action: &str,
    table: &str,
    id: i64,
    data: Value,
) -> Result<()> {
    let mut entry = renox::audit::Entry::new(action)
        .subject(table, id)
        .data(data);
    if let Some(user) = user {
        entry = entry.user(user);
    }
    renox::audit::record(&state.db, entry).await?;
    Ok(())
}

/// The rows of `item_id`: the preferred supplier first, then the
/// alternatives by rank.
fn ordered(mut rows: Vec<ItemSupplier>) -> Vec<ItemSupplier> {
    rows.sort_by_key(|row| (row.role != SupplierRole::Preferred, row.rank, row.id));
    rows
}

/// The rows of `item_id`, read in `tx`.
pub async fn rows_in(tx: &mut Transaction, item_id: i64) -> Result<Vec<ItemSupplier>> {
    Ok(ordered(
        ItemSupplier::query()
            .where_(ItemSupplier::ITEM_ID.eq(item_id))
            .get(&mut *tx)
            .await?,
    ))
}

/// The rows of `item_id`: the preferred supplier first, then the
/// alternatives by rank.
pub async fn rows_of(db: &Db, item_id: i64) -> Result<Vec<ItemSupplier>> {
    Ok(ordered(
        ItemSupplier::query()
            .where_(ItemSupplier::ITEM_ID.eq(item_id))
            .get(db)
            .await?,
    ))
}

/// Ranks the alternatives of `item_id` 1..n in `tx`. `moved` is a row that
/// goes to rank `wanted` (the others make room); without it the order stays.
async fn renumber(tx: &mut Transaction, item_id: i64, moved: Option<(i64, i64)>) -> Result<()> {
    let mut alternatives: Vec<ItemSupplier> = rows_in(tx, item_id)
        .await?
        .into_iter()
        .filter(|row| row.role == SupplierRole::Alternative)
        .collect();
    if let Some((id, wanted)) = moved
        && let Some(at) = alternatives.iter().position(|row| row.id == id)
    {
        let row = alternatives.remove(at);
        let to = usize::try_from((wanted - 1).max(0))
            .unwrap_or(0)
            .min(alternatives.len());
        alternatives.insert(to, row);
    }
    let now = renox::db::now();
    for (index, row) in alternatives.iter().enumerate() {
        let rank = i64::try_from(index + 1).unwrap_or(i64::MAX);
        if row.rank != rank {
            ItemSupplier::query()
                .where_(ItemSupplier::ID.eq(row.id))
                .update(&mut *tx, &[("rank", &rank), ("updated_at", &now)])
                .await?;
        }
    }
    Ok(())
}

/// What the purchaser enters for a supplier of an item.
#[derive(Debug, Clone, Default)]
pub struct NewItemSupplier {
    /// The item.
    pub item_id: i64,
    /// The supplier party.
    pub supplier: i64,
    /// Preferred or alternative.
    pub role: SupplierRole,
    /// The rank among the alternatives; the last one when none.
    pub rank: Option<i64>,
    /// The supplier's own code for the item.
    pub supplier_item_code: Option<String>,
    /// The unit the supplier sells in.
    pub purchase_unit: Option<String>,
    /// Days from order to delivery.
    pub lead_time_days: i64,
    /// The smallest order, in thousandths of the base unit.
    pub min_order_qty_milli: i64,
    /// The document kinds the supplier should hold for the item.
    pub certificate_kinds: Vec<String>,
    /// A free note.
    pub notes: Option<String>,
}

impl NewItemSupplier {
    /// `supplier` as the preferred supplier of `item_id`.
    pub fn preferred(item_id: i64, supplier: PartyId) -> Self {
        NewItemSupplier {
            item_id,
            supplier: supplier.0,
            role: SupplierRole::Preferred,
            ..NewItemSupplier::default()
        }
    }

    /// `supplier` as an alternative of `item_id`, at `rank` when given.
    pub fn alternative(item_id: i64, supplier: PartyId, rank: Option<i64>) -> Self {
        NewItemSupplier {
            item_id,
            supplier: supplier.0,
            role: SupplierRole::Alternative,
            rank,
            ..NewItemSupplier::default()
        }
    }

    /// With a lead time of `days`.
    pub fn lead_time(mut self, days: i64) -> Self {
        self.lead_time_days = days;
        self
    }

    /// With a minimum order of `quantity_milli`.
    pub fn minimum(mut self, quantity_milli: i64) -> Self {
        self.min_order_qty_milli = quantity_milli;
        self
    }

    /// With the document kinds the supplier should hold.
    pub fn certificates(mut self, kinds: &[&str]) -> Self {
        self.certificate_kinds = kinds.iter().map(|kind| (*kind).to_owned()).collect();
        self
    }
}

/// Lists a supplier of an item (`item_suppliers.manage`).
///
/// # Errors
///
/// `Forbidden` without the permission; 422 when the supplier is already on
/// the item, or when a preferred supplier is asked for that needs a request
/// (the item has one, or ever had one).
pub async fn add(state: &AppState, user: &AuthUser, new: NewItemSupplier) -> Result<ItemSupplier> {
    require(state, user, permissions::ITEM_SUPPLIERS_MANAGE).await?;
    let item = Item::find_or_404(&state.db, new.item_id).await?;
    if new.lead_time_days < 0 || new.min_order_qty_milli < 0 {
        return Err(refused(said("wl_inventory.supplier.negative", &[])));
    }
    let mut tx = state.db.begin().await?;
    let rows = rows_in(&mut tx, item.id).await?;
    if rows.iter().any(|row| row.supplier_party_id == new.supplier) {
        return Err(refused(said("wl_inventory.supplier.twice", &[])));
    }
    let preferred = new.role == SupplierRole::Preferred;
    if preferred
        && (item.had_preferred_supplier
            || rows.iter().any(|row| row.role == SupplierRole::Preferred))
    {
        return Err(refused(said("wl_inventory.supplier.needs_request", &[])));
    }
    let mut row = ItemSupplier {
        item_id: item.id,
        supplier_party_id: new.supplier,
        role: new.role,
        rank: if preferred { 0 } else { i64::from(i32::MAX) },
        supplier_item_code: new.supplier_item_code.clone(),
        purchase_unit: new.purchase_unit.clone(),
        lead_time_days: new.lead_time_days,
        min_order_qty_milli: new.min_order_qty_milli,
        active: true,
        certificate_kinds: new.certificate_kinds.join(","),
        notes: new.notes.clone(),
        ..ItemSupplier::default()
    };
    row.insert(&mut tx).await?;
    if preferred {
        Item::query()
            .where_(Item::ID.eq(item.id))
            .update(&mut tx, &[("had_preferred_supplier", &true)])
            .await?;
    } else {
        renumber(&mut tx, item.id, new.rank.map(|rank| (row.id, rank))).await?;
    }
    tx.commit().await?;
    audit(
        state,
        Some(user.id),
        "inventory.item_supplier_added",
        ItemSupplier::TABLE,
        row.id,
        json!({
            "item": item.id,
            "supplier": new.supplier,
            "role": new.role.as_str(),
        }),
    )
    .await?;
    ItemSupplier::find_or_404(&state.db, row.id).await
}

/// What [`deactivate`] did.
#[derive(Debug)]
pub enum DeactivateOutcome {
    /// The alternative is not in use any more.
    Deactivated(ItemSupplier),
    /// The row is the preferred supplier: a request was opened (or the one
    /// still open is returned) and the row stays as it is.
    Requested(PreferredSupplierRequest),
}

/// Takes row `row_id` out of use (`item_suppliers.manage`). An alternative
/// is deactivated at once; the preferred supplier's row waits for a second
/// person (`STK.ACL.07`).
///
/// # Errors
///
/// `Forbidden` without the permission; 422 for a row that is not in use.
pub async fn deactivate(
    state: &AppState,
    user: &AuthUser,
    row_id: i64,
) -> Result<DeactivateOutcome> {
    require(state, user, permissions::ITEM_SUPPLIERS_MANAGE).await?;
    let row = ItemSupplier::find_or_404(&state.db, row_id).await?;
    if !row.active {
        return Err(refused(said("wl_inventory.supplier.inactive", &[])));
    }
    if row.role == SupplierRole::Preferred {
        let open = PreferredSupplierRequest::query()
            .where_(PreferredSupplierRequest::ITEM_ID.eq(row.item_id))
            .where_(PreferredSupplierRequest::STATUS.eq(RequestStatus::Pending))
            .first(&state.db)
            .await?;
        if let Some(open) = open {
            return Ok(DeactivateOutcome::Requested(open));
        }
        let mut request = PreferredSupplierRequest {
            item_id: row.item_id,
            from_supplier_party_id: row.supplier_party_id,
            kind: "deactivate".to_owned(),
            status: RequestStatus::Pending,
            requested_by: Some(user.id),
            ..PreferredSupplierRequest::default()
        };
        request.save(&state.db).await?;
        audit(
            state,
            Some(user.id),
            "inventory.preferred_supplier_requested",
            PreferredSupplierRequest::TABLE,
            request.id,
            json!({ "item": row.item_id, "supplier": row.supplier_party_id, "kind": "deactivate" }),
        )
        .await?;
        return Ok(DeactivateOutcome::Requested(request));
    }
    let now = renox::db::now();
    ItemSupplier::query()
        .where_(ItemSupplier::ID.eq(row.id))
        .update(&state.db, &[("active", &false), ("updated_at", &now)])
        .await?;
    audit(
        state,
        Some(user.id),
        "inventory.item_supplier_deactivated",
        ItemSupplier::TABLE,
        row.id,
        json!({ "item": row.item_id, "supplier": row.supplier_party_id }),
    )
    .await?;
    Ok(DeactivateOutcome::Deactivated(
        ItemSupplier::find_or_404(&state.db, row.id).await?,
    ))
}

/// The last price of an item from a supplier: per base unit, excluding VAT,
/// in the currency of the document it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastPrice {
    /// Fils of one base unit.
    pub unit_price: i64,
    /// The ISO currency code of the document.
    pub currency: String,
}

/// The last price of `item_id` from `supplier`: the unit price of the latest
/// posted receipt line (accepted quantity above 0, against an order), else of
/// the latest issued order line. `None` when the supplier never delivered or
/// was ordered from.
///
/// # Errors
///
/// A database error.
pub async fn last_price(db: &Db, item_id: i64, supplier: i64) -> Result<Option<LastPrice>> {
    last_price_before(db, item_id, supplier, None, None).await
}

/// [`last_price`] as it stood before `before`, leaving out the order
/// `except_order` and its receipts: the price a new order line is compared
/// with by the supplier score. With `before`, only receipts posted before
/// it and orders issued before it count.
///
/// # Errors
///
/// A database error.
pub async fn last_price_before(
    db: &Db,
    item_id: i64,
    supplier: i64,
    before: Option<DateTime>,
    except_order: Option<i64>,
) -> Result<Option<LastPrice>> {
    let mut receipts = GoodsReceipt::query()
        .where_(GoodsReceipt::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(GoodsReceipt::STATUS.eq(GoodsReceiptStatus::Posted))
        .get(db)
        .await?;
    receipts.sort_by_key(|receipt| (receipt.posted_at, receipt.id));
    for receipt in receipts.iter().rev() {
        let Some(order_id) = receipt.purchase_order_id else {
            continue;
        };
        if except_order == Some(order_id)
            || before.is_some_and(|before| receipt.posted_at.is_none_or(|at| at >= before))
        {
            continue;
        }
        let lines = GoodsReceiptLine::query()
            .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt.id))
            .where_(GoodsReceiptLine::ITEM_ID.eq(item_id))
            .get(db)
            .await?;
        if let Some(line) = lines.iter().rev().find(|line| line.accepted_qty_milli > 0) {
            let order = PurchaseOrder::find_or_404(db, order_id).await?;
            return Ok(Some(LastPrice {
                unit_price: line.unit_cost,
                currency: order.currency,
            }));
        }
    }
    let mut orders = PurchaseOrder::query()
        .where_(PurchaseOrder::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(PurchaseOrder::STATUS.is_in([
            PurchaseOrderStatus::Issued,
            PurchaseOrderStatus::PartiallyReceived,
            PurchaseOrderStatus::Received,
            PurchaseOrderStatus::Closed,
        ]))
        .get(db)
        .await?;
    orders.sort_by_key(|order| order.id);
    for order in orders.iter().rev() {
        if except_order == Some(order.id)
            || before.is_some_and(|before| order.issued_at.is_none_or(|at| at >= before))
        {
            continue;
        }
        let lines = PurchaseOrderLine::query()
            .where_(PurchaseOrderLine::PURCHASE_ORDER_ID.eq(order.id))
            .where_(PurchaseOrderLine::ITEM_ID.eq(item_id))
            .get(db)
            .await?;
        if let Some(line) = lines.last() {
            return Ok(Some(LastPrice {
                unit_price: line.unit_price,
                currency: order.currency.clone(),
            }));
        }
    }
    Ok(None)
}

/// Whether `supplier` is refused a new agreement now (`BlockGuard`, `start`).
/// The reason is never read: a blocked supplier is *on hold*.
///
/// # Errors
///
/// A database error.
pub async fn is_on_hold(state: &AppState, supplier: i64) -> Result<bool> {
    let mut tx = state.db.begin().await?;
    let outcome = guard(state)
        .check_in(&mut tx, None, PartyId(supplier), ActionKind::Start)
        .await?;
    tx.rollback().await?;
    Ok(!outcome.is_allowed())
}

/// The document kinds of `row` that are not valid today for its supplier: a
/// warning, never a block.
///
/// # Errors
///
/// A database error.
pub async fn warnings_of(state: &AppState, row: &ItemSupplier) -> Result<Vec<String>> {
    let subject = row.supplier().subject();
    let docs = documents::guard(state);
    let today = documents::today();
    let mut warnings = Vec::new();
    for kind in row.kinds() {
        if !docs
            .is_valid(&state.db, &subject, &kind, today)
            .await?
            .is_valid()
        {
            warnings.push(kind);
        }
    }
    Ok(warnings)
}

/// Whether `a` is a stronger row than `b` on one item: the preferred one,
/// else the better (lower) rank.
fn stronger(a: &ItemSupplier, b: &ItemSupplier) -> bool {
    match (a.role, b.role) {
        (SupplierRole::Preferred, SupplierRole::Alternative) => true,
        (SupplierRole::Alternative, SupplierRole::Preferred) => false,
        _ => a.rank < b.rank,
    }
}

/// `PartiesMerged`: the rows of the merged supplier move to the survivor;
/// where both are on one item the stronger row stays.
///
/// # Errors
///
/// A database error.
pub(crate) async fn on_merged(state: &AppState, event: &PartiesMerged) -> Result<()> {
    let moved = ItemSupplier::query()
        .where_(ItemSupplier::SUPPLIER_PARTY_ID.eq(event.merged.0))
        .get(&state.db)
        .await?;
    for row in moved {
        let mut tx = state.db.begin().await?;
        let rows = rows_in(&mut tx, row.item_id).await?;
        let twin = rows
            .iter()
            .find(|other| other.supplier_party_id == event.survivor.0)
            .cloned();
        let now = renox::db::now();
        match twin {
            None => {
                ItemSupplier::query()
                    .where_(ItemSupplier::ID.eq(row.id))
                    .update(
                        &mut tx,
                        &[
                            ("supplier_party_id", &event.survivor.0),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
            }
            Some(twin) if stronger(&row, &twin) => {
                ItemSupplier::query()
                    .where_(ItemSupplier::ID.eq(twin.id))
                    .delete(&mut tx)
                    .await?;
                ItemSupplier::query()
                    .where_(ItemSupplier::ID.eq(row.id))
                    .update(
                        &mut tx,
                        &[
                            ("supplier_party_id", &event.survivor.0),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
            }
            Some(_) => {
                ItemSupplier::query()
                    .where_(ItemSupplier::ID.eq(row.id))
                    .delete(&mut tx)
                    .await?;
            }
        }
        renumber(&mut tx, row.item_id, None).await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: SupplierRole, rank: i64) -> ItemSupplier {
        ItemSupplier {
            role,
            rank,
            active: true,
            ..ItemSupplier::default()
        }
    }

    #[test]
    fn the_preferred_row_is_stronger_then_the_better_rank() {
        let preferred = row(SupplierRole::Preferred, 0);
        let first = row(SupplierRole::Alternative, 1);
        let second = row(SupplierRole::Alternative, 2);
        assert!(stronger(&preferred, &first));
        assert!(!stronger(&first, &preferred));
        assert!(stronger(&first, &second));
        assert!(!stronger(&second, &first));
    }

    #[test]
    fn the_rows_are_read_preferred_first_then_by_rank() {
        let mut a = row(SupplierRole::Alternative, 2);
        a.id = 1;
        let mut b = row(SupplierRole::Preferred, 0);
        b.id = 2;
        let mut c = row(SupplierRole::Alternative, 1);
        c.id = 3;
        let ids: Vec<i64> = ordered(vec![a, b, c]).iter().map(|r| r.id).collect();
        assert_eq!(ids, [2, 3, 1]);
    }

    #[test]
    fn the_certificate_kinds_are_split_and_trimmed() {
        let mut item = row(SupplierRole::Alternative, 1);
        item.certificate_kinds = "halal_certificate, iso_9001,".to_owned();
        assert_eq!(item.kinds(), ["halal_certificate", "iso_9001"]);
        item.certificate_kinds = String::new();
        assert!(item.kinds().is_empty());
    }

    #[test]
    fn a_role_is_parsed_from_its_key() {
        assert_eq!(
            SupplierRole::parse("preferred"),
            Some(SupplierRole::Preferred)
        );
        assert_eq!(
            SupplierRole::parse(" alternative "),
            Some(SupplierRole::Alternative)
        );
        assert_eq!(SupplierRole::parse("other"), None);
        assert_eq!(SupplierRole::Preferred.as_str(), "preferred");
    }
}
