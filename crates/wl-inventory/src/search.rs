//! Items, purchase orders, goods receipts, lots and dispatches in the search
//! index and the *Related* panel (BL-AGT-11, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! Identifiers: an item's code (SKU), the PO, GRN and dispatch numbers, a
//! dispatch's courier tracking number, a lot's number and the supplier's lot
//! number. Two lots of different items may share a number: both match.
//! Stock movements and lines are not indexed one by one; they are reached
//! through *Related* (a lot's movements, holds, recalls and the dispatches
//! that shipped it).
//!
//! The documents are written in the transaction that writes the record: a
//! goods receipt's and a dispatch's creation, a lot's creation (receipt,
//! production, batch), and every transition of the lot, purchase order,
//! goods receipt and dispatch machines ([`reindex_lot`] and siblings). Items
//! and purchase orders are inserted by the apps and the demo seeds, which
//! end with a full rebuild; adding a supplier to an item re-indexes it.
//!
//! Decisions: the module has no asset entity, no barcode and no serial
//! number on its records yet, so those are not indexed (they join the item's
//! document when the module stores them). A dispatch is found by holders of
//! `stock.view` (the permission its page reads besides `dispatch.manage`).
//! The supplier bills of a receipt are declared by the invoicing module,
//! whose tables hold the link.

use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::Transition;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    DISPATCH_SUBJECT_TYPE, GOODS_RECEIPT_SUBJECT_TYPE, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    PURCHASE_ORDER_SUBJECT_TYPE, RECALL_SUBJECT_TYPE, STOCK_HOLD_SUBJECT_TYPE,
};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::dispatch::Dispatch;
use crate::goods_receipt::GoodsReceipt;
use crate::item::Item;
use crate::location::Location;
use crate::lot::Lot;
use crate::permissions;
use crate::purchase_order::PurchaseOrder;

/// The subject type of a stock movement (reached through a lot, never
/// indexed on its own).
pub const MOVEMENT_SUBJECT_TYPE: &str = "inventory.stock_movement";

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";

/// Registers the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Item>(ITEM_SUBJECT_TYPE)
        .permission(permissions::ITEMS_VIEW)
        .label("Items")
        .link("/items/{id}/suppliers")
        .document(item_document)
        .register();
    Searchable::of::<PurchaseOrder>(PURCHASE_ORDER_SUBJECT_TYPE)
        .permission(permissions::PURCHASE_ORDERS_VIEW)
        .label("Purchase orders")
        .document_in(|tx, order| Box::pin(purchase_order_document(tx, order)))
        .register();
    Searchable::of::<GoodsReceipt>(GOODS_RECEIPT_SUBJECT_TYPE)
        .permission(permissions::PURCHASE_ORDERS_VIEW)
        .label("Goods receipts")
        .link("/goods-receipts/{id}")
        .document(receipt_document)
        .register();
    Searchable::of::<Lot>(LOT_SUBJECT_TYPE)
        .permission(permissions::STOCK_VIEW)
        .label("Lots")
        .link("/stock/lots/{id}")
        .document_in(|tx, lot| Box::pin(lot_document(tx, lot)))
        .register();
    Searchable::of::<Dispatch>(DISPATCH_SUBJECT_TYPE)
        .permission(permissions::STOCK_VIEW)
        .label("Dispatches")
        .link("/dispatches/{id}")
        .document(dispatch_document)
        .register();
    register_relations();
}

fn register_relations() {
    // item → its lots and suppliers.
    Relation::has_many(
        ITEM_SUBJECT_TYPE,
        "lots",
        LOT_SUBJECT_TYPE,
        "wl_inventory_lots",
        "item_id",
    )
    .label("Lots")
    .register();
    Relation::new(ITEM_SUBJECT_TYPE, "suppliers", PARTY_SUBJECT_TYPE)
        .label("Suppliers")
        .permission(PARTIES_VIEW)
        .link("/parties/{id}")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT item_id AS from_id, supplier_party_id AS to_id \
                 FROM wl_inventory_item_suppliers WHERE active = ? AND item_id IN ",
                vec![DbValue::Bool(true)],
                (ITEM_SUBJECT_TYPE, PARTY_SUBJECT_TYPE),
            )
        })
        .register();

    // purchase order → its supplier and receipts.
    Relation::belongs_to(
        PURCHASE_ORDER_SUBJECT_TYPE,
        "supplier",
        PARTY_SUBJECT_TYPE,
        "wl_inventory_purchase_orders",
        "supplier_party_id",
    )
    .label("Supplier")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::has_many(
        PURCHASE_ORDER_SUBJECT_TYPE,
        "receipts",
        GOODS_RECEIPT_SUBJECT_TYPE,
        "wl_inventory_goods_receipts",
        "purchase_order_id",
    )
    .label("Goods receipts")
    .expand()
    .register();

    // goods receipt → its order and the lots it brought in.
    Relation::belongs_to(
        GOODS_RECEIPT_SUBJECT_TYPE,
        "purchase_order",
        PURCHASE_ORDER_SUBJECT_TYPE,
        "wl_inventory_goods_receipts",
        "purchase_order_id",
    )
    .label("Purchase order")
    .register();
    Relation::new(GOODS_RECEIPT_SUBJECT_TYPE, "lots", LOT_SUBJECT_TYPE)
        .label("Lots")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT source_id AS from_id, id AS to_id FROM wl_inventory_lots \
                 WHERE source_type = ? AND source_id IN ",
                vec![DbValue::Text(GOODS_RECEIPT_SUBJECT_TYPE.to_owned())],
                (GOODS_RECEIPT_SUBJECT_TYPE, LOT_SUBJECT_TYPE),
            )
        })
        .register();

    // lot → its item, movements, holds, recalls and the dispatches that
    // shipped it.
    Relation::belongs_to(
        LOT_SUBJECT_TYPE,
        "item",
        ITEM_SUBJECT_TYPE,
        "wl_inventory_lots",
        "item_id",
    )
    .label("Item")
    .register();
    Relation::has_many(
        LOT_SUBJECT_TYPE,
        "movements",
        MOVEMENT_SUBJECT_TYPE,
        "wl_inventory_stock_movements",
        "lot_id",
    )
    .label("Stock movements")
    .permission(permissions::STOCK_VIEW)
    .register();
    Relation::new(LOT_SUBJECT_TYPE, "holds", STOCK_HOLD_SUBJECT_TYPE)
        .label("Holds")
        .permission(permissions::STOCK_VIEW)
        .link("/stock/holds")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT DISTINCT lot_id AS from_id, stock_hold_id AS to_id \
                 FROM wl_inventory_stock_hold_lines WHERE lot_id IN ",
                Vec::new(),
                (LOT_SUBJECT_TYPE, STOCK_HOLD_SUBJECT_TYPE),
            )
        })
        .register();
    Relation::new(LOT_SUBJECT_TYPE, "recalls", RECALL_SUBJECT_TYPE)
        .label("Recalls")
        .permission(permissions::RECALLS_VIEW)
        .link("/recalls/{id}")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT DISTINCT lot_id AS from_id, recall_id AS to_id \
                 FROM wl_inventory_recall_lots WHERE lot_id IN ",
                Vec::new(),
                (LOT_SUBJECT_TYPE, RECALL_SUBJECT_TYPE),
            )
        })
        .register();
    Relation::new(LOT_SUBJECT_TYPE, "dispatches", DISPATCH_SUBJECT_TYPE)
        .label("Dispatches that shipped it")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT DISTINCT l.lot_id AS from_id, l.dispatch_id AS to_id \
                 FROM wl_inventory_dispatch_lines l \
                 JOIN wl_inventory_dispatches d ON d.id = l.dispatch_id \
                 WHERE d.status NOT IN (?, ?, ?) AND l.lot_id IN ",
                vec![
                    DbValue::Text("draft".to_owned()),
                    DbValue::Text("picked".to_owned()),
                    DbValue::Text("cancelled".to_owned()),
                ],
                (LOT_SUBJECT_TYPE, DISPATCH_SUBJECT_TYPE),
            )
        })
        .register();

    // dispatch → its lots and the sales order it serves (its deliveries are
    // declared by the delivery module, from its table).
    Relation::new(DISPATCH_SUBJECT_TYPE, "lots", LOT_SUBJECT_TYPE)
        .label("Lots")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT DISTINCT dispatch_id AS from_id, lot_id AS to_id \
                 FROM wl_inventory_dispatch_lines WHERE lot_id IS NOT NULL AND dispatch_id IN ",
                Vec::new(),
                (DISPATCH_SUBJECT_TYPE, LOT_SUBJECT_TYPE),
            )
        })
        .register();
    // The sales order's dispatches, from this module's table.
    Relation::new("sales.sales_order", "dispatches", DISPATCH_SUBJECT_TYPE)
        .label("Dispatches")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT source_id AS from_id, id AS to_id FROM wl_inventory_dispatches \
                 WHERE source_type = ? AND source_id IN ",
                vec![DbValue::Text("sales.sales_order".to_owned())],
                ("sales.sales_order", DISPATCH_SUBJECT_TYPE),
            )
        })
        .register();
    Relation::new(DISPATCH_SUBJECT_TYPE, "order", "sales.sales_order")
        .label("Sales order")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT id AS from_id, source_id AS to_id FROM wl_inventory_dispatches \
                 WHERE source_type = ? AND id IN ",
                vec![DbValue::Text("sales.sales_order".to_owned())],
                (DISPATCH_SUBJECT_TYPE, "sales.sales_order"),
            )
        })
        .register();
}

/// Runs `statement` (ending in `IN `) with `values` then one placeholder per
/// id, reading `from_id` and `to_id` as records of the two `types`.
fn pairs_of<'a>(
    db: &'a Db,
    ids: &'a [i64],
    statement: &'static str,
    values: Vec<DbValue>,
    types: (&'static str, &'static str),
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut bound = values;
        bound.extend(ids.iter().map(|id| DbValue::Integer(*id)));
        let rows = sql(format!("{statement}({placeholders})"))
            .bind_all(bound)
            .fetch_all(db)
            .await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let from: Option<i64> = row.try_get("from_id")?;
            let to: Option<i64> = row.try_get("to_id")?;
            if let (Some(from), Some(to)) = (from, to) {
                pairs.push((Subject::new(types.0, from), Subject::new(types.1, to)));
            }
        }
        Ok(pairs)
    })
}

/// How an item reads in a search result.
fn item_document(item: &Item) -> SearchDocument {
    let mut document = SearchDocument::new(&item.name_en)
        .secondary(&item.code)
        .identifier(kinds::SKU, &item.code);
    if let Some(arabic) = &item.name_ar {
        document = document.keyword(arabic);
    }
    document
}

/// The scope of location `id`: its project, building or branch.
async fn location_scope(tx: &mut Transaction, id: i64) -> Result<renox::auth::permissions::Scope> {
    Ok(Location::query()
        .where_eq("id", id)
        .first(&mut *tx)
        .await?
        .map(|location| location.scope())
        .unwrap_or_else(renox::auth::permissions::Scope::global))
}

/// How a purchase order reads in a search result; a draft has no number yet.
async fn purchase_order_document(
    tx: &mut Transaction,
    order: &PurchaseOrder,
) -> Result<SearchDocument> {
    let title = order
        .number
        .clone()
        .unwrap_or_else(|| format!("Draft purchase order #{}", order.id));
    let mut document = SearchDocument::new(title)
        .secondary(
            order
                .expected_date
                .map(|date| format!("Expected {date}"))
                .unwrap_or_default(),
        )
        .status(order.status.as_str())
        .scope(location_scope(tx, order.location_id).await?);
    if let Some(number) = &order.number {
        document = document.identifier(kinds::NUMBER, number);
    }
    Ok(document)
}

/// How a goods receipt reads in a search result.
fn receipt_document(receipt: &GoodsReceipt) -> SearchDocument {
    let title = receipt
        .number
        .clone()
        .unwrap_or_else(|| format!("Draft goods receipt #{}", receipt.id));
    let mut document = SearchDocument::new(title)
        .secondary(receipt.delivery_note.clone().unwrap_or_default())
        .status(receipt.status.as_str())
        .scope(receipt.scope());
    if let Some(number) = &receipt.number {
        document = document.identifier(kinds::NUMBER, number);
    }
    if let Some(note) = &receipt.delivery_note {
        document = document.keyword(note);
    }
    document
}

/// How a lot reads in a search result: its number, the item it is of.
async fn lot_document(tx: &mut Transaction, lot: &Lot) -> Result<SearchDocument> {
    let item = Item::query()
        .where_eq("id", lot.item_id)
        .first(&mut *tx)
        .await?;
    let item_words = item
        .as_ref()
        .map(|item| format!("{} · {}", item.code, item.name_en))
        .unwrap_or_default();
    let secondary = match lot.expiry_date {
        Some(expiry) => format!("{item_words} · expires {expiry}"),
        None => item_words,
    };
    let mut document = SearchDocument::new(&lot.number)
        .secondary(secondary)
        .identifier(kinds::LOT, &lot.number)
        .status(lot.status.as_str());
    if let Some(supplier_lot) = &lot.supplier_lot {
        document = document.identifier(kinds::LOT, supplier_lot);
    }
    if let Some(item) = &item {
        document = document.keyword(&item.name_en);
    }
    Ok(document)
}

/// How a dispatch reads in a search result.
fn dispatch_document(dispatch: &Dispatch) -> SearchDocument {
    let title = dispatch
        .number
        .clone()
        .unwrap_or_else(|| format!("Draft dispatch #{}", dispatch.id));
    let mut document = SearchDocument::new(title)
        .secondary(&dispatch.recipient_name)
        .status(dispatch.status.as_str())
        .scope(dispatch.scope());
    if let Some(number) = &dispatch.number {
        document = document.identifier(kinds::NUMBER, number);
    }
    if let Some(tracking) = &dispatch.tracking_number {
        document = document.identifier(kinds::TRACKING, tracking);
    }
    document
}

/// Writes the document of `subject_type` record `id` in `tx`.
pub(crate) async fn index(tx: &mut Transaction, subject_type: &str, id: i64) -> Result {
    index_in(tx, &Subject::new(subject_type, id)).await?;
    Ok(())
}

/// `transition`, writing the record's document after its own effects, in
/// its transaction.
fn reindexed<M>(transition: Transition<M>, subject_type: &'static str) -> Transition<M>
where
    M: renox::state_machine::StateMachine + Model<Key = i64> + Send + Sync + 'static,
{
    transition.effect(move |record: &M, _ctx, tx| {
        let id = record.id();
        Box::pin(async move { index(tx, subject_type, id).await })
    })
}

/// A lot transition that keeps the lot's document current.
pub(crate) fn reindex_lot(transition: Transition<Lot>) -> Transition<Lot> {
    reindexed(transition, LOT_SUBJECT_TYPE)
}

/// A purchase order transition that keeps the order's document current.
pub(crate) fn reindex_order(transition: Transition<PurchaseOrder>) -> Transition<PurchaseOrder> {
    reindexed(transition, PURCHASE_ORDER_SUBJECT_TYPE)
}

/// A goods receipt transition that keeps the receipt's document current.
pub(crate) fn reindex_receipt(transition: Transition<GoodsReceipt>) -> Transition<GoodsReceipt> {
    reindexed(transition, GOODS_RECEIPT_SUBJECT_TYPE)
}

/// A dispatch transition that keeps the dispatch's document current.
pub(crate) fn reindex_dispatch(transition: Transition<Dispatch>) -> Transition<Dispatch> {
    reindexed(transition, DISPATCH_SUBJECT_TYPE)
}
