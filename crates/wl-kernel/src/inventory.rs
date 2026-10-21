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

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::invoicing::SupplierDebitNoteId;
use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of an item.
pub const ITEM_SUBJECT_TYPE: &str = "inventory.item";

/// The subject type of a purchase request.
pub const PURCHASE_REQUEST_SUBJECT_TYPE: &str = "inventory.purchase_request";

/// The subject type of a stock count.
pub const STOCK_COUNT_SUBJECT_TYPE: &str = "inventory.stock_count";

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

/// The subject type of a landed cost.
pub const LANDED_COST_SUBJECT_TYPE: &str = "inventory.landed_cost";

/// The subject type of a landed cost adjustment.
pub const LANDED_COST_ADJUSTMENT_SUBJECT_TYPE: &str = "inventory.landed_cost_adjustment";

/// The subject type of a Return to supplier.
pub const SUPPLIER_RETURN_SUBJECT_TYPE: &str = "inventory.supplier_return";

/// The subject type of a stock hold.
pub const STOCK_HOLD_SUBJECT_TYPE: &str = "inventory.stock_hold";

/// The subject type of a transfer between two locations.
pub const TRANSFER_SUBJECT_TYPE: &str = "inventory.transfer";

/// The subject type of a pending issue.
pub const PENDING_ISSUE_SUBJECT_TYPE: &str = "inventory.pending_issue";

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

/// One line a revision of a purchase order changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseOrderRevisedLine {
    /// The purchase order line.
    pub po_line: i64,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The quantity before, in thousandths of the base unit.
    pub old_quantity_milli: i64,
    /// The quantity after.
    pub new_quantity_milli: i64,
    /// The price of one base unit before, in fils.
    pub old_unit_price: i64,
    /// The price of one base unit after.
    pub new_unit_price: i64,
}

/// A purchase order was revised (wl-inventory §6 `PurchaseOrderRevised`): the
/// new revision is in force, the committed cost follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurchaseOrderRevised {
    /// The order, `inventory.purchase_order:3`.
    pub purchase_order: Subject,
    /// The supplier.
    pub supplier: PartyId,
    /// The revision now in force (2 for the first revision).
    pub revision: i64,
    /// The currency of the order; the totals are in its minor units.
    pub currency: String,
    /// The lines whose quantity or price changed.
    pub lines: Vec<PurchaseOrderRevisedLine>,
    /// The total of the earlier revision, excluding VAT.
    pub old_total: i64,
    /// The total of the new revision, excluding VAT.
    pub new_total: i64,
    /// Why it was revised.
    pub reason: String,
}

impl Event for PurchaseOrderRevised {}

/// What a landed cost put on one item at one location: the part that
/// re-valued the stock still on hand and the part that went to the landed
/// cost variance (or the cost object charged on receipt) because the goods
/// were already consumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedCostShare {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// Where the goods were received, `inventory.location:2`.
    pub location: Subject,
    /// The whole share of the item, in fils (the two parts below add up).
    pub amount: i64,
    /// The part that re-valued stock still on hand.
    pub revalued: i64,
    /// The part on goods already consumed.
    pub variance: i64,
}

/// A landed cost was allocated over its receipts (wl-inventory §6
/// `LandedCostAllocated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedCostAllocated {
    /// The landed cost, `inventory.landed_cost:7`.
    pub landed_cost: Subject,
    /// The receipts it was spread over.
    pub receipts: Vec<Subject>,
    /// The total spread, in fils.
    pub amount: i64,
    /// The share of every item.
    pub shares: Vec<LandedCostShare>,
}

impl Event for LandedCostAllocated {}

/// A landed cost adjustment was posted (wl-inventory §6 `LandedCostAdjusted`):
/// a late or corrected import cost spread on the original basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedCostAdjusted {
    /// The landed cost, `inventory.landed_cost:7`.
    pub landed_cost: Subject,
    /// The adjustment, `inventory.landed_cost_adjustment:3`.
    pub adjustment: Subject,
    /// The total of the adjustment, in fils; negative for a refund.
    pub amount: i64,
    /// The share of every item: what re-valued stock and what went to the
    /// variance.
    pub shares: Vec<LandedCostShare>,
}

impl Event for LandedCostAdjusted {}

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

/// A purchase order that is still open, as a vertical lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOrder {
    /// The order, `inventory.purchase_order:3`.
    pub order: Subject,
    /// The number, `PO-2026-0001`.
    pub number: Option<String>,
    /// The supplier.
    pub supplier: PartyId,
    /// Whether any quantity was accepted on it: such an order is
    /// short-closed, an untouched one is cancelled.
    pub received: bool,
}

/// The future a [`ScopeStock`] method returns.
pub type ScopeFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What a vertical does with the purchasing and the stock of one scope (a
/// construction project, wl-inventory §3 *Location* scope link) when the
/// scope stops: lists its open orders, cancels its open purchase requests
/// and opens the count of its stores (02 Construction §7.1 *Project*
/// `terminate`). `wl-inventory` provides it with [`provide_scope_stock`];
/// without that module [`scope_stock`] gives [`NoScopeStock`], which has
/// nothing to list, cancel or count.
pub trait ScopeStock: Send + Sync + 'static {
    /// The purchase orders delivering to a location in `scope` that can still
    /// receive goods (`issued` or `partially_received`), oldest first.
    fn open_orders<'a>(
        &'a self,
        _state: &'a AppState,
        _scope: &'a Subject,
    ) -> ScopeFuture<'a, Vec<OpenOrder>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Cancels, with `reason`, the purchase requests of a location in `scope`
    /// that are `draft`, `submitted` or `approved` and have no purchase order
    /// issued. Returns them.
    fn cancel_open_requests<'a>(
        &'a self,
        _state: &'a AppState,
        _scope: &'a Subject,
        _reason: &'a str,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Opens a stock count (`STK.CNT.01`) of every location in `scope` that
    /// has no count open, with `reason`. Returns the counts opened now.
    fn open_stock_counts<'a>(
        &'a self,
        _state: &'a AppState,
        _scope: &'a Subject,
        _reason: &'a str,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Whether any goods receipt was recorded at a location in `scope`
    /// (a project that was cancelled may have none). The default is no.
    fn has_goods_receipts<'a>(
        &'a self,
        _state: &'a AppState,
        _scope: &'a Subject,
    ) -> ScopeFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The stock counts of the locations in `scope` that are not `approved`
    /// or `cancelled` yet.
    fn open_counts<'a>(
        &'a self,
        _state: &'a AppState,
        _scope: &'a Subject,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The scope stock when `inventory` is off: nothing to list, cancel or count.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoScopeStock;

impl ScopeStock for NoScopeStock {}

/// Makes `stock` the app's [`ScopeStock`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_scope_stock(app: &mut Registry, stock: impl ScopeStock) {
    let stock: Arc<dyn ScopeStock> = Arc::new(stock);
    app.provide(stock);
}

/// The app's [`ScopeStock`], or [`NoScopeStock`] when none is provided.
pub fn scope_stock(state: &AppState) -> Arc<dyn ScopeStock> {
    match state.provided::<Arc<dyn ScopeStock>>() {
        Some(stock) => Arc::clone(&stock),
        None => Arc::new(NoScopeStock),
    }
}

/// A supplier's monthly snapshot is under the threshold (wl-inventory §6
/// `SupplierScoreBelowThreshold`, `STK.RUL.09`): procurement is asked to
/// review the supplier. `not_enough_data` never fires it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierScoreBelowThreshold {
    /// The supplier.
    pub supplier: PartyId,
    /// First day of the window the snapshot covers.
    pub window_from: NaiveDate,
    /// Last day of the window.
    pub window_to: NaiveDate,
    /// The score, 0 to 100.
    pub score: i64,
    /// The receipts counted.
    pub receipts: i64,
    /// The on-time share, in basis points.
    pub on_time_bp: Option<i64>,
    /// The rejected share of the value received, in basis points.
    pub rejected_bp: Option<i64>,
    /// The quality incidents.
    pub incidents: i64,
    /// The price variance, in basis points.
    pub price_variance_bp: Option<i64>,
}

impl Event for SupplierScoreBelowThreshold {}

/// Another supplier was made the preferred one of an item, or the preferred
/// row was taken out of use (wl-inventory §6 `PreferredSupplierChanged`,
/// `STK.ACL.07`): emitted once the change is applied, after the second
/// person approved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferredSupplierChanged {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The preferred supplier before; none when the item had none.
    pub old_supplier: Option<PartyId>,
    /// The preferred supplier now; none when the preferred row was removed.
    pub new_supplier: Option<PartyId>,
    /// The user who asked for the change.
    pub requester: Option<i64>,
    /// The user who approved it (the requester when the second person is
    /// switched off).
    pub approver: Option<i64>,
}

impl Event for PreferredSupplierChanged {}

/// One quality incident of a supplier, as the scorecard counts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierIncident {
    /// The record that reported it, `manufacturing.ncr:12`.
    pub reference: Subject,
    /// The supplier whose goods caused it.
    pub supplier: PartyId,
    /// The day it happened (Asia/Dubai).
    pub on: NaiveDate,
}

/// Quality incidents per supplier and date for the supplier scorecard
/// (wl-inventory §8 `SupplierIncidentSource`). A vertical provides it with
/// [`provide_supplier_incidents`]: 07 reports the NCRs on incoming lots. Only
/// incidents whose cause is the supplier and that are not voided are
/// answered, so a later void or change of cause drops an incident out of the
/// next answer.
pub trait SupplierIncidentSource: Send + Sync + 'static {
    /// The incidents of `supplier` on days from `from` to `to`, both
    /// included.
    fn incidents<'a>(
        &'a self,
        db: &'a Db,
        supplier: PartyId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> MatchFuture<'a, Vec<SupplierIncident>>;
}

/// The source when no vertical provides one: no incidents.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSupplierIncidents;

impl SupplierIncidentSource for NoSupplierIncidents {
    fn incidents<'a>(
        &'a self,
        _db: &'a Db,
        _supplier: PartyId,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> MatchFuture<'a, Vec<SupplierIncident>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Makes `source` the app's [`SupplierIncidentSource`].
pub fn provide_supplier_incidents(app: &mut Registry, source: impl SupplierIncidentSource) {
    let source: Arc<dyn SupplierIncidentSource> = Arc::new(source);
    app.provide(source);
}

/// The app's [`SupplierIncidentSource`], or [`NoSupplierIncidents`] when none
/// is provided.
pub fn supplier_incidents(state: &AppState) -> Arc<dyn SupplierIncidentSource> {
    match state.provided::<Arc<dyn SupplierIncidentSource>>() {
        Some(source) => Arc::clone(&source),
        None => Arc::new(NoSupplierIncidents),
    }
}

/// A purchase order as a vertical's rule sees it when the order is issued or
/// revised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderFacts {
    /// The order, `inventory.purchase_order:3`.
    pub order: Subject,
    /// The supplier.
    pub supplier: PartyId,
    /// The scope of the delivery location (a project), if it has one.
    pub scope: Option<Subject>,
    /// The order's date: the day it is issued or revised, in Asia/Dubai.
    pub on_date: NaiveDate,
    /// The transition asked: `issue`, `revise` or `revise_part`.
    pub transition: String,
}

/// An approval a vertical's rule adds to issuing or revising an order
/// (`CON.ACL.13`): the action key of an approval policy the vertical
/// registers (the policy has to match), and the steps that decide it. The
/// requester never approves (segregation of duties); the order stays as it is
/// until the last step approves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderApproval {
    /// The action key of the vertical's approval policy.
    pub action: String,
    /// The steps that decide the request, in order.
    pub steps: Vec<crate::approvals::Step>,
}

/// Rules a vertical adds to the purchase order (wl-inventory §4 *Purchase
/// order* `issue` and `revise`). A vertical provides it with
/// [`provide_order_rules`]; `wl-inventory` asks it after the block guard
/// passed and before the transition runs, so an order to a blocked supplier
/// never gets a request of its own.
pub trait PurchaseOrderRules: Send + Sync + 'static {
    /// The approval the order needs before `facts.transition` runs, if any.
    /// The default asks for none.
    fn approval_for<'a>(
        &'a self,
        _state: &'a AppState,
        _facts: &'a OrderFacts,
    ) -> MatchFuture<'a, Option<OrderApproval>> {
        Box::pin(async { Ok(None) })
    }
}

/// The rules when no vertical provides any: none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOrderRules;

impl PurchaseOrderRules for NoOrderRules {}

/// Makes `rules` the app's [`PurchaseOrderRules`].
pub fn provide_order_rules(app: &mut Registry, rules: impl PurchaseOrderRules) {
    let rules: Arc<dyn PurchaseOrderRules> = Arc::new(rules);
    app.provide(rules);
}

/// The app's [`PurchaseOrderRules`], or [`NoOrderRules`] when none is
/// provided.
pub fn order_rules(state: &AppState) -> Arc<dyn PurchaseOrderRules> {
    match state.provided::<Arc<dyn PurchaseOrderRules>>() {
        Some(rules) => Arc::clone(&rules),
        None => Arc::new(NoOrderRules),
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
    fn the_revised_event_round_trips_through_json() {
        let revised = PurchaseOrderRevised {
            purchase_order: Subject::new(PURCHASE_ORDER_SUBJECT_TYPE, 3),
            supplier: PartyId(7),
            revision: 2,
            currency: "AED".into(),
            lines: vec![PurchaseOrderRevisedLine {
                po_line: 11,
                item: Subject::new(ITEM_SUBJECT_TYPE, 9),
                old_quantity_milli: 10_000,
                new_quantity_milli: 10_000,
                old_unit_price: 90_000,
                new_unit_price: 120_000,
            }],
            old_total: 900_000,
            new_total: 1_200_000,
            reason: "price rise".into(),
        };
        let json = renox::serde_json::to_string(&revised).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<PurchaseOrderRevised>(&json).unwrap(),
            revised
        );
    }

    #[test]
    fn the_landed_cost_events_round_trip_through_json() {
        let share = LandedCostShare {
            item: Subject::new(ITEM_SUBJECT_TYPE, 9),
            location: Subject::new(LOCATION_SUBJECT_TYPE, 2),
            amount: 100_000,
            revalued: 60_000,
            variance: 40_000,
        };
        let adjusted = LandedCostAdjusted {
            landed_cost: Subject::new(LANDED_COST_SUBJECT_TYPE, 7),
            adjustment: Subject::new(LANDED_COST_ADJUSTMENT_SUBJECT_TYPE, 3),
            amount: 100_000,
            shares: vec![share.clone()],
        };
        let json = renox::serde_json::to_string(&adjusted).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<LandedCostAdjusted>(&json).unwrap(),
            adjusted
        );
        let allocated = LandedCostAllocated {
            landed_cost: Subject::new(LANDED_COST_SUBJECT_TYPE, 7),
            receipts: vec![Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, 5)],
            amount: 100_000,
            shares: vec![share],
        };
        let json = renox::serde_json::to_string(&allocated).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<LandedCostAllocated>(&json).unwrap(),
            allocated
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

/// One line of a stock hold: a quantity of an item at a location, of a lot
/// when the item is lot-tracked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldLine {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The location the quantity is at, `inventory.location:2`.
    pub location: Subject,
    /// The lot, `inventory.lot:4`.
    pub lot: Option<Subject>,
    /// The quantity held, in thousandths of the base unit.
    pub quantity_milli: i64,
}

/// A stock hold was placed (wl-inventory §6 `StockHoldPlaced`): the quantity
/// in its scope is blocked until someone decides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockHoldPlaced {
    /// The hold, `inventory.stock_hold:3`.
    pub hold: Subject,
    /// Its number, `HLD-2026-0001`.
    pub number: String,
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    pub reason: String,
    /// The record that caused it: a recall, a ticket, an NCR, a reading.
    pub reference: Option<Subject>,
    /// The lines the hold was placed with.
    pub lines: Vec<HeldLine>,
}

impl Event for StockHoldPlaced {}

/// A whole lot was put on hold (wl-inventory §6 `LotHeld`, §4 *Lot* `hold`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotHeld {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// Why: the reason of the hold.
    pub reason: String,
    /// What is left of the lot in all locations, in thousandths.
    pub quantity_milli: i64,
}

impl Event for LotHeld {}

/// Automatic consumption was short, or a balance went below zero (wl-inventory
/// §6 `StockNegative`): a pending issue was recorded for the short quantity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockNegative {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The location, `inventory.location:2`.
    pub location: Subject,
    /// The short quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The record that consumed: a visit, a sale, a work order.
    pub source: Option<Subject>,
    /// The pending issue that holds the short quantity.
    pub pending_issue: Option<Subject>,
}

impl Event for StockNegative {}

/// The future a [`StockService`] or [`StockHoldService`] method returns.
pub type StockFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Consumption another module posts automatically (wl-inventory §3 *Negative
/// stock*, the `ConsumptionSource` extension): a clinic visit, an F&B sale, a
/// work order backflush.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumeRequest {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The location it is taken from, `inventory.location:2`.
    pub location: Subject,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The record that consumes: a visit, a sale, a work order.
    pub source: Subject,
    /// The record the cost is charged to.
    pub cost_object: Option<Subject>,
    /// When the consumption happened; now when `None`.
    pub at: Option<renox::db::DateTime>,
}

impl ConsumeRequest {
    /// `quantity_milli` of `item` taken from `location` for `source`.
    pub fn new(item: Subject, location: Subject, quantity_milli: i64, source: Subject) -> Self {
        ConsumeRequest {
            item,
            location,
            quantity_milli,
            source,
            cost_object: None,
            at: None,
        }
    }

    /// Charged to `cost_object`.
    pub fn cost_object(mut self, cost_object: Subject) -> Self {
        self.cost_object = Some(cost_object);
        self
    }

    /// Happened at `at`.
    pub fn at(mut self, at: renox::db::DateTime) -> Self {
        self.at = Some(at);
        self
    }
}

/// What a consumption did: the part that posted now, and the pending issue
/// for the short part.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Consumed {
    /// The quantity that posted now, in thousandths.
    pub posted_milli: i64,
    /// The quantity that became a pending issue, in thousandths.
    pub short_milli: i64,
    /// The pending issue, `inventory.pending_issue:5`, when part was short.
    pub pending_issue: Option<Subject>,
}

/// The stock service other modules call (wl-inventory §6 `StockService`).
/// Without the inventory module [`stock_service`] gives [`NoStockService`],
/// which posts nothing.
pub trait StockService: Send + Sync + 'static {
    /// Consumes automatically: posts what is available and records a pending
    /// issue for the short part, so the business event is never blocked.
    fn consume<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ConsumeRequest,
    ) -> StockFuture<'a, Consumed> {
        Box::pin(async { Ok(Consumed::default()) })
    }

    /// The `source` that consumed (a visit, a sale) was cancelled before its
    /// pending issues posted: each one still open is cleared with the reason
    /// *source cancelled*, without a movement. Returns how many were cleared.
    fn source_cancelled<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
    ) -> StockFuture<'a, usize> {
        Box::pin(async { Ok(0) })
    }
}

/// The stock service when `inventory` is off: nothing is posted.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStockService;

impl StockService for NoStockService {}

/// Makes `service` the app's [`StockService`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_stock_service(app: &mut Registry, service: impl StockService) {
    let service: Arc<dyn StockService> = Arc::new(service);
    app.provide(service);
}

/// The app's [`StockService`], or [`NoStockService`] when none is provided.
pub fn stock_service(state: &AppState) -> Arc<dyn StockService> {
    match state.provided::<Arc<dyn StockService>>() {
        Some(service) => Arc::clone(&service),
        None => Arc::new(NoStockService),
    }
}

/// A hold a vertical automation (06: the out-of-range reading) places on the
/// stock at one location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceHold {
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    pub reason: String,
    /// The record that caused it: a checklist reading, a ticket.
    pub reference: Subject,
    /// The location whose stock is held, `inventory.location:2`.
    pub location: Subject,
    /// The items held; every item at the location when empty.
    pub items: Vec<Subject>,
}

/// The stock hold service (wl-inventory §6 `StockHoldService`). Without the
/// inventory module [`stock_hold_service`] gives [`NoStockHoldService`],
/// which holds nothing.
pub trait StockHoldService: Send + Sync + 'static {
    /// Places a hold on the stock at a location as the system. Returns the
    /// hold, or `None` when the module is off.
    fn place<'a>(
        &'a self,
        _state: &'a AppState,
        _hold: PlaceHold,
    ) -> StockFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }
}

/// The hold service when `inventory` is off: nothing is held.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStockHoldService;

impl StockHoldService for NoStockHoldService {}

/// Makes `service` the app's [`StockHoldService`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_stock_hold_service(app: &mut Registry, service: impl StockHoldService) {
    let service: Arc<dyn StockHoldService> = Arc::new(service);
    app.provide(service);
}

/// The app's [`StockHoldService`], or [`NoStockHoldService`] when none is
/// provided.
pub fn stock_hold_service(state: &AppState) -> Arc<dyn StockHoldService> {
    match state.provided::<Arc<dyn StockHoldService>>() {
        Some(service) => Arc::clone(&service),
        None => Arc::new(NoStockHoldService),
    }
}
