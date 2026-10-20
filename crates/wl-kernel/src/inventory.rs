//! The inventory contract (wl-inventory §6): what other modules and the
//! verticals may know about stock, purchase orders and goods receipts
//! without depending on the `wl-inventory` crate.
//!
//! `wl-inventory` emits [`SupplierReturnDispatched`] when goods go back to a
//! supplier, and [`GoodsReceived`] when a goods receipt is posted,
//! with the accepted and the rejected quantity of every line, and provides a
//! [`PurchaseMatchSource`]: the purchase order lines with the quantity
//! accepted so far, which wl-invoicing matches a supplier bill against
//! (`FIN.PAB.01`). Rejected goods never count as received.
//!
//! Quantities are thousandths of the item's base unit (1000 = 1 kg, 1 pcs),
//! money is fils of the purchase order's currency.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::SupplierDebitNoteId;
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of an item.
pub const ITEM_SUBJECT_TYPE: &str = "inventory.item";

/// The subject type of a stock location.
pub const LOCATION_SUBJECT_TYPE: &str = "inventory.location";

/// The subject type of a purchase order.
pub const PURCHASE_ORDER_SUBJECT_TYPE: &str = "inventory.purchase_order";

/// The subject type of a purchase order line.
pub const PURCHASE_ORDER_LINE_SUBJECT_TYPE: &str = "inventory.purchase_order_line";

/// The subject type of a goods receipt.
pub const GOODS_RECEIPT_SUBJECT_TYPE: &str = "inventory.goods_receipt";

/// The subject type of a lot.
pub const LOT_SUBJECT_TYPE: &str = "inventory.lot";

/// The subject type of a Return to supplier.
pub const SUPPLIER_RETURN_SUBJECT_TYPE: &str = "inventory.supplier_return";

/// One line of a posted goods receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodsReceivedLine {
    /// The purchase order line it was received on; `None` for a free
    /// replacement.
    pub po_line: Option<i64>,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// What arrived, in thousandths of the base unit.
    pub received_qty_milli: i64,
    /// What entered stock and counts on the order, in thousandths.
    pub accepted_qty_milli: i64,
    /// What was refused, in thousandths; never in stock.
    pub rejected_qty_milli: i64,
    /// Why it was refused (`temperature`, `quality`...); `None` when nothing
    /// was.
    pub reason: Option<String>,
    /// The value of the accepted quantity at the order's price, in fils.
    pub value: i64,
}

/// A goods receipt was posted (wl-inventory §6): stock, the order and the
/// bill match took the accepted quantity only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoodsReceived {
    /// The receipt, `inventory.goods_receipt:5`.
    pub receipt: Subject,
    /// The purchase order, `inventory.purchase_order:3`, if any.
    pub purchase_order: Option<Subject>,
    /// The supplier.
    pub supplier: PartyId,
    /// The location it was received at, `inventory.location:2`.
    pub location: Subject,
    /// The lines.
    pub lines: Vec<GoodsReceivedLine>,
    /// The value of everything accepted, in fils.
    pub value: i64,
}

impl Event for GoodsReceived {}

/// What the user wants for goods sent back to a supplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnOutcome {
    /// A credit: the supplier issues a credit note (wl-invoicing drafts the
    /// supplier debit note).
    Credit,
    /// New goods in place of the returned ones.
    Replace,
}

/// One line of a dispatched Return to supplier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierReturnDispatchedLine {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot, `inventory.lot:4`, if any.
    pub lot: Option<Subject>,
    /// What went back, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The purchase order line, if any.
    pub po_line: Option<i64>,
    /// The order's price of one base unit, in fils; the unit cost of the
    /// line when it names no order line.
    pub po_price: i64,
    /// Credit or replacement.
    pub wanted_outcome: ReturnOutcome,
    /// Why the goods went back.
    pub reason: String,
}

/// A Return to supplier was dispatched (wl-inventory §6): the goods left with
/// the supplier or their driver. wl-invoicing drafts a supplier debit note
/// for the `credit` lines at the order's price (`FIN.RUL.17`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierReturnDispatched {
    /// The return, `inventory.supplier_return:4`.
    pub supplier_return: Subject,
    /// The supplier.
    pub supplier: PartyId,
    /// The record the goods came from: the goods receipt, a stock hold...
    pub source: Subject,
    /// The lines.
    pub lines: Vec<SupplierReturnDispatchedLine>,
}

impl Event for SupplierReturnDispatched {}

/// A Return to supplier was credited (wl-inventory §6 `credit`): the
/// supplier's debit notes and replacements cover every line, or procurement
/// settled it by hand with a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierReturnCredited {
    /// The return, `inventory.supplier_return:4`.
    pub supplier_return: Subject,
    /// The supplier.
    pub supplier: PartyId,
    /// How: `debit_note`, `replacement`, `refund_received` or
    /// `supplier_refused`.
    pub basis: String,
    /// The debit note that completed the cover, if one did.
    pub debit_note: Option<SupplierDebitNoteId>,
    /// The reason of a manual credit.
    pub reason: Option<String>,
    /// The value posted as wastage because the supplier refused the credit,
    /// in fils; 0 otherwise.
    pub wastage: i64,
}

impl Event for SupplierReturnCredited {}

/// One purchase order line, as the supplier bill match reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchLine {
    /// The purchase order line.
    pub po_line: i64,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// What was ordered, in thousandths of the base unit.
    pub ordered_qty_milli: i64,
    /// What was accepted so far, in thousandths; rejected goods are not in it.
    pub accepted_qty_milli: i64,
    /// The price of one base unit, in fils.
    pub unit_price: i64,
}

/// The future a [`PurchaseMatchSource`] method returns.
pub type MatchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The purchase order and goods-received lines for the supplier bill match
/// (wl-inventory §6). `wl-inventory` provides it with
/// [`provide_purchase_match`]; without that module [`purchase_match`] gives
/// [`NoPurchaseMatch`], which knows no order.
pub trait PurchaseMatchSource: Send + Sync + 'static {
    /// The lines of `purchase_order` (`inventory.purchase_order:3`) with
    /// their accepted quantity; empty for an unknown order.
    fn lines<'a>(
        &'a self,
        _db: &'a Db,
        _purchase_order: &'a Subject,
    ) -> MatchFuture<'a, Vec<MatchLine>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The match source when `inventory` is off: no order has lines.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPurchaseMatch;

impl PurchaseMatchSource for NoPurchaseMatch {}

/// Makes `source` the app's [`PurchaseMatchSource`]; `wl-inventory` calls it
/// in `register`.
pub fn provide_purchase_match(app: &mut Registry, source: impl PurchaseMatchSource) {
    let source: Arc<dyn PurchaseMatchSource> = Arc::new(source);
    app.provide(source);
}

/// The app's [`PurchaseMatchSource`], or [`NoPurchaseMatch`] when none is
/// provided.
pub fn purchase_match(state: &AppState) -> Arc<dyn PurchaseMatchSource> {
    match state.provided::<Arc<dyn PurchaseMatchSource>>() {
        Some(source) => Arc::clone(&source),
        None => Arc::new(NoPurchaseMatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_types_are_inventory_ones() {
        for subject_type in [
            ITEM_SUBJECT_TYPE,
            LOCATION_SUBJECT_TYPE,
            PURCHASE_ORDER_SUBJECT_TYPE,
            GOODS_RECEIPT_SUBJECT_TYPE,
            SUPPLIER_RETURN_SUBJECT_TYPE,
            LOT_SUBJECT_TYPE,
        ] {
            assert_eq!(Subject::new(subject_type, 1).subject_type, subject_type);
        }
    }

    #[test]
    fn the_dispatched_event_round_trips_through_json() {
        let dispatched = SupplierReturnDispatched {
            supplier_return: Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, 4),
            supplier: PartyId(7),
            source: Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, 5),
            lines: vec![SupplierReturnDispatchedLine {
                item: Subject::new(ITEM_SUBJECT_TYPE, 9),
                lot: Some(Subject::new(LOT_SUBJECT_TYPE, 2)),
                quantity_milli: 10_000,
                po_line: Some(11),
                po_price: 5_000,
                wanted_outcome: ReturnOutcome::Credit,
                reason: "damaged".into(),
            }],
        };
        let json = renox::serde_json::to_string(&dispatched).unwrap();
        assert!(json.contains("\"credit\""), "{json}");
        assert_eq!(
            renox::serde_json::from_str::<SupplierReturnDispatched>(&json).unwrap(),
            dispatched
        );
    }

    #[test]
    fn the_received_event_round_trips_through_json() {
        let received = GoodsReceived {
            receipt: Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, 5),
            purchase_order: Some(Subject::new(PURCHASE_ORDER_SUBJECT_TYPE, 3)),
            supplier: PartyId(7),
            location: Subject::new(LOCATION_SUBJECT_TYPE, 2),
            lines: vec![GoodsReceivedLine {
                po_line: Some(11),
                item: Subject::new(ITEM_SUBJECT_TYPE, 9),
                received_qty_milli: 100_000,
                accepted_qty_milli: 80_000,
                rejected_qty_milli: 20_000,
                reason: Some("temperature".into()),
                value: 96_000,
            }],
            value: 96_000,
        };
        let json = renox::serde_json::to_string(&received).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<GoodsReceived>(&json).unwrap(),
            received
        );
    }
}
