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

/// The subject type of one line of a stock hold.
pub const STOCK_HOLD_LINE_SUBJECT_TYPE: &str = "inventory.stock_hold_line";

/// The subject type of a recall.
pub const RECALL_SUBJECT_TYPE: &str = "inventory.recall";

/// The subject type of a transfer between two locations.
pub const TRANSFER_SUBJECT_TYPE: &str = "inventory.transfer";

/// The subject type of an affected line of a recall.
pub const RECALL_LINE_SUBJECT_TYPE: &str = "inventory.recall_line";

/// The subject type of a pending issue.
pub const PENDING_ISSUE_SUBJECT_TYPE: &str = "inventory.pending_issue";

/// The subject type of a dispatch.
pub const DISPATCH_SUBJECT_TYPE: &str = "inventory.dispatch";

/// The subject type of a dispatch line (what `wl-delivery` hands back as the
/// source line of a delivery's item).
pub const DISPATCH_LINE_SUBJECT_TYPE: &str = "inventory.dispatch_line";

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
    fn a_traced_holder_round_trips_through_json_with_its_mark() {
        let holder = TracedHolder {
            line: Subject::new(RECALL_LINE_SUBJECT_TYPE, 7),
            subject: Some(Subject::new("clinics.visit", 3)),
            party: Some(PartyId(11)),
            lot: Some(Subject::new(LOT_SUBJECT_TYPE, 9)),
            location: None,
            quantity_milli: 2_000,
            on: None,
            kind: "possible".into(),
            mark: TraceMark::Possible,
            sales: 1,
            delivery_status: None,
            notice: "pending".into(),
        };
        let json = renox::serde_json::to_string(&holder).unwrap();
        assert!(json.contains("\"possible\""), "{json}");
        assert_eq!(
            renox::serde_json::from_str::<TracedHolder>(&json).unwrap(),
            holder
        );
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

/// A pending issue was `posted` against stock that arrived or was `cleared`
/// (wl-inventory §3 *Pending issue*): it is no longer open. The record that
/// consumed (a work order) learns what its short part became: the lot and
/// the cost it posted with, and whether other pending issues of the same
/// source are still open (07 Manufacturing, `MFG.RUL`: the line's supervisor
/// is asked to resume a work order held for material once none is).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingIssueClosed {
    /// The pending issue that closed, `inventory.pending_issue:5`. A partly
    /// covered pending issue is split: the covered part is a row of its own,
    /// the rest stays open on [`PendingIssueClosed::original`].
    pub pending_issue: Subject,
    /// The pending issue the source was given by `consume`
    /// ([`Consumed::pending_issue`]): the same as `pending_issue`, or the one
    /// it was split from.
    pub original: Subject,
    /// The record that consumed: a visit, a sale, a work order.
    pub source: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The location, `inventory.location:2`.
    pub location: Subject,
    /// How it closed: `posted` (stock arrived), `cleared` (the storekeeper
    /// entered the stock that had been used) or `source_cancelled` (nothing
    /// was consumed).
    pub how: String,
    /// The lot it posted against, for a lot-tracked item.
    pub lot: Option<Subject>,
    /// The quantity that closed, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one base unit it posted at, in fils; 0 when nothing posted.
    pub unit_cost: i64,
    /// How many pending issues of the same source are still open.
    pub open_for_source: i64,
}

impl Event for PendingIssueClosed {}

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
    /// Derived from a recipe or a bill of materials without a scan (06: an
    /// F&B sale): the lot it posts against keeps the lot balances right but
    /// is no proof of what was used, so a recall trace lists the subject as
    /// *possible* ([`TraceMark::Possible`]).
    #[serde(default)]
    pub theoretical: bool,
    /// The lot (`inventory.lot:4`) the user picked or scanned: it is taken
    /// first, the rest of a larger quantity follows first expiry first out.
    /// A lot that cannot be taken (held, not released, empty) is skipped.
    #[serde(default)]
    pub lot: Option<Subject>,
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
            theoretical: false,
            lot: None,
        }
    }

    /// Takes from `lot` first (the lot the user picked or scanned).
    pub fn lot(mut self, lot: Subject) -> Self {
        self.lot = Some(lot);
        self
    }

    /// Derived from a recipe or bill of materials, not scanned.
    pub fn theoretical(mut self) -> Self {
        self.theoretical = true;
        self
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

/// The part of a consumption that posted against one lot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumedLot {
    /// The lot, `inventory.lot:4`; `None` for an item that is not
    /// lot-tracked.
    pub lot: Option<Subject>,
    /// The quantity taken from it, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one base unit it was posted at, in fils (the item's
    /// average cost then): a work order keeps it per issue to value what
    /// goes back or is scrapped.
    #[serde(default)]
    pub unit_cost: i64,
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
    /// The lots the posted part came from, first expiry first.
    #[serde(default)]
    pub lots: Vec<ConsumedLot>,
}

/// A lot that can be taken from now (`STK.MOV.01`, `STK.LOT.01`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockLot {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// Its number, as printed on the pack.
    pub number: String,
    /// The last day it can be used.
    pub expiry_date: Option<NaiveDate>,
    /// What can be taken from it at the location, in thousandths: not held,
    /// not reserved.
    pub available_milli: i64,
}

/// What can be taken of an item at a location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockChoice {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// Whether the item is tracked by lot.
    pub lot_tracked: bool,
    /// The lots that can be taken from, first expiry first: released, not
    /// held, not expired, with a quantity. Empty for an item that is not
    /// lot-tracked.
    pub lots: Vec<StockLot>,
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

    /// The store of the scope `kind`/`id` (a branch): the location the scope
    /// issues from, `inventory.location:2`. `None` when it has none.
    fn store_of<'a>(
        &'a self,
        _state: &'a AppState,
        _scope_kind: &'a str,
        _scope_id: i64,
    ) -> StockFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }

    /// What can be taken of the item with `code` at `location`: whether it is
    /// lot-tracked and, if so, its lots first expiry first (the lot an issue
    /// screen proposes is the first). `None` when no such item exists.
    fn choices<'a>(
        &'a self,
        _state: &'a AppState,
        _code: &'a str,
        _location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        Box::pin(async { Ok(None) })
    }

    /// What can be taken of `item` (`inventory.item:9`) at `location`, as
    /// [`StockService::choices`] tells it by item code: for a caller that
    /// holds the item's id (a work order). `None` when no such item exists
    /// or the module is off.
    fn choices_of_item<'a>(
        &'a self,
        _state: &'a AppState,
        _item: &'a Subject,
        _location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        Box::pin(async { Ok(None) })
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

/// One component of what a sale line consumes: an item and how much of it
/// goes into one unit of the line (a recipe ingredient).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineComponent {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The quantity for one unit of the sale line, in thousandths of the
    /// item's base unit.
    pub per_unit_milli: i64,
}

/// What the stock side of a sale line is, for a sales cancellation outcome
/// *scrap* (BL-STK-26): the vertical knows the recipe and whether the
/// kitchen's consumption was already posted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleLineStock {
    /// The location the components are taken from, `inventory.location:2`.
    pub location: Subject,
    /// What one unit of the line consumes.
    pub components: Vec<LineComponent>,
    /// The quantity of the line, in thousandths of its unit: what the
    /// cancellation of a whole order scraps.
    pub quantity_milli: i64,
    /// Whether the consumption of the line was already posted (so a scrap
    /// posts no second movement and the line is reported as wastage value).
    pub consumed: bool,
    /// The value of the line's goods, in fils: what is reported as wastage
    /// when the consumption was already posted.
    pub value: i64,
}

/// The stock side of the lines of a sales order. The vertical that explodes a
/// recipe provides it (06: the menu item's recipe); the default knows no
/// recipe, so a scrapped line without picked goods posts nothing.
pub trait SaleLineStockSource: Send + Sync + 'static {
    /// The stock side of the sale line `line` of `order` (`sales.sales_order`),
    /// or `None` when it has none.
    fn of_line<'a>(
        &'a self,
        _state: &'a AppState,
        _order: &'a Subject,
        _line: i64,
    ) -> StockFuture<'a, Option<SaleLineStock>> {
        Box::pin(async { Ok(None) })
    }
}

/// The source when nothing provides one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSaleLineStock;

impl SaleLineStockSource for NoSaleLineStock {}

/// Makes `source` the app's [`SaleLineStockSource`].
pub fn provide_sale_line_stock(app: &mut Registry, source: impl SaleLineStockSource) {
    let source: Arc<dyn SaleLineStockSource> = Arc::new(source);
    app.provide(source);
}

/// The app's [`SaleLineStockSource`], or [`NoSaleLineStock`].
pub fn sale_line_stock(state: &AppState) -> Arc<dyn SaleLineStockSource> {
    match state.provided::<Arc<dyn SaleLineStockSource>>() {
        Some(source) => Arc::clone(&source),
        None => Arc::new(NoSaleLineStock),
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

    /// The hold with its lines and what was decided for them; `None` when
    /// the module is off or the hold is unknown.
    fn view<'a>(
        &'a self,
        _state: &'a AppState,
        _hold: &'a Subject,
    ) -> StockFuture<'a, Option<HoldView>> {
        Box::pin(async { Ok(None) })
    }

    /// The stock locations of the deployment, by code: what a vertical lets
    /// its user pick when a unit (a fridge) is a location of its own. Empty
    /// when the module is off.
    fn locations<'a>(&'a self, _state: &'a AppState) -> StockFuture<'a, Vec<LocationInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// `user` decides a line of a hold (`release`, `discard` or
    /// `return_to_supplier`). The module's own rules apply: the permission in
    /// the location's scope, a second person for a listed reason or a
    /// discard above the wastage value. `None` when the module is off.
    fn decide<'a>(
        &'a self,
        _state: &'a AppState,
        _user: &'a AuthUser,
        _line: &'a Subject,
        _outcome: &'a str,
    ) -> StockFuture<'a, Option<HoldDecision>> {
        Box::pin(async { Ok(None) })
    }
}

/// A stock location, as another module reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationInfo {
    /// The location, `inventory.location:2`.
    pub location: Subject,
    /// Its code, e.g. `OUT1-F2`.
    pub code: String,
    /// Its name.
    pub name: String,
}

/// One line of a stock hold, as another module reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldLineView {
    /// The line, `inventory.stock_hold_line:7`.
    pub line: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The item's code.
    pub item_code: String,
    /// The item's name.
    pub item_name: String,
    /// The location the quantity is at, `inventory.location:2`.
    pub location: Subject,
    /// The lot, `inventory.lot:4`.
    pub lot: Option<Subject>,
    /// The quantity held, in thousandths.
    pub quantity_milli: i64,
    /// The value at the item's average cost, in fils.
    pub value: i64,
    /// `undecided`, `released`, `discarded` or `returned_to_supplier`.
    pub status: String,
}

/// A stock hold with its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldView {
    /// The hold, `inventory.stock_hold:3`.
    pub hold: Subject,
    /// Its number, `HLD-2026-0001`.
    pub number: String,
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    pub reason: String,
    /// Whether the hold is still `open`.
    pub open: bool,
    /// The lines, oldest first.
    pub lines: Vec<HoldLineView>,
}

/// What deciding a line of a hold did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldDecision {
    /// The line is decided.
    Decided,
    /// A second person must approve first; the line stays `undecided`.
    Waiting,
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

/// What was decided for one line of a closed stock hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldOutcome {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The location the quantity was at, `inventory.location:2`.
    pub location: Subject,
    /// The lot, `inventory.lot:4`.
    pub lot: Option<Subject>,
    /// The quantity held, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// `release`, `discard` or `return_to_supplier`.
    pub outcome: String,
}

/// A stock hold closed because its last line was decided (wl-inventory §6
/// `StockHoldClosed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockHoldClosed {
    /// The hold, `inventory.stock_hold:3`.
    pub hold: Subject,
    /// Its number, `HLD-2026-0001`.
    pub number: String,
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    pub reason: String,
    /// The record that caused it.
    pub reference: Option<Subject>,
    /// What was decided for each line.
    pub outcomes: Vec<HeldOutcome>,
}

impl Event for StockHoldClosed {}

/// A lot left its hold and can be used again (wl-inventory §6 `LotReleased`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotReleased {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
}

impl Event for LotReleased {}

/// A lot was discarded (wl-inventory §6 `LotDiscarded`): what was left of it
/// went to wastage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotDiscarded {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// Why: the hold or expiry reason.
    pub reason: String,
    /// The quantity of the lot that went to wastage, in thousandths: what
    /// the lines of the hold discarded and what the lot's own discard wrote
    /// off after them.
    pub quantity_milli: i64,
}

impl Event for LotDiscarded {}

/// A recall was opened (wl-inventory §6 `RecallOpened`): every lot in its
/// scope is on hold, wherever it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallOpened {
    /// The recall, `inventory.recall:2`.
    pub recall: Subject,
    /// Its number, `RCL-2026-0001`.
    pub number: String,
    /// `supplier`, `authority`, `internal` or `complaint`.
    pub source: String,
    /// Why it was opened.
    pub reason: String,
    /// The lots in its scope, `inventory.lot:4`.
    pub lots: Vec<Subject>,
    /// The stock hold that blocks them, `inventory.stock_hold:3`.
    pub stock_hold: Subject,
}

impl Event for RecallOpened {}

/// How sure a recall trace is that a subject got the recalled lot (§4
/// *Recall*, forward trace).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceMark {
    /// A movement of the lot to the subject proves it.
    Confirmed,
    /// A pending issue whose lot was assigned when stock arrived: it may
    /// have been any lot of the item at the location (*lot uncertain*).
    Uncertain,
    /// Theoretical consumption, or an item not tracked by lot: the unit used
    /// cannot be proven to come from the recalled lot.
    Possible,
}

/// The party behind a subject in a recall trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceParty {
    /// The patient, customer, vehicle owner or fleet of the subject.
    Party(PartyId),
    /// A sale without a known party (walk-in POS): the trace lists a count
    /// per outlet and day instead of a line per sale.
    WalkIn,
    /// The vertical does not know: the line is listed with the party
    /// *unknown* for manual handling.
    Unknown,
    /// The goods of the subject are back on site (a returned or cancelled
    /// dispatch): the subject is not listed (07 §4.4 step 8).
    NotAffected,
}

/// The future a [`TracePartyResolver`] or [`LotTrace`] method returns.
pub type TraceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The party behind a subject in a recall trace (wl-inventory §6
/// `TracePartyResolver`), provided by the vertical: 05 visit to patient, 06
/// order to customer, 07 sales order or dispatch to customer, 08 job card to
/// vehicle owner or fleet. Without one [`trace_party_resolver`] gives
/// [`NoTraceParties`]: every party is unknown.
pub trait TracePartyResolver: Send + Sync + 'static {
    /// The party of `subject`.
    fn party<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
    ) -> TraceFuture<'a, TraceParty> {
        Box::pin(async { Ok(TraceParty::Unknown) })
    }

    /// The delivery status of a dispatched `subject` (`wl-delivery`), when
    /// the deployment has one: an undelivered delivery can still be stopped.
    /// The trace also asks it for a dispatch that has not left and for a
    /// production source (07: [`NOT_SHIPPED`], [`SHIPPED`]).
    fn delivery_status<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
    ) -> TraceFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    /// Who is told about a line whose party is `party`: one recipient per
    /// active buyer of a customer, else the party itself, reached through its
    /// contact points (07: the customer's main contact email). The default is
    /// the party.
    fn addressees<'a>(
        &'a self,
        _state: &'a AppState,
        party: PartyId,
    ) -> TraceFuture<'a, Vec<crate::notify::NotifyRecipient>> {
        Box::pin(async move { Ok(vec![crate::notify::NotifyRecipient::Party { id: party }]) })
    }
}

/// The delivery status of a subject whose goods have not left the plant: a
/// dispatch that is `draft` or `picked`, or a work order for an order not yet
/// delivered (07 §4.4 step 8).
pub const NOT_SHIPPED: &str = "not_shipped";

/// The delivery status of a dispatch that left and is on its way (07 lists it
/// for the recall owner to bring back).
pub const SHIPPED: &str = "shipped";

/// The delivery status of a dispatch the customer accepted.
pub const DELIVERED: &str = "delivered";

/// The resolver when no vertical provides one: nobody is known.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTraceParties;

impl TracePartyResolver for NoTraceParties {}

/// Makes `resolver` the app's [`TracePartyResolver`]; a vertical calls it in
/// `register`.
pub fn provide_trace_party_resolver(app: &mut Registry, resolver: impl TracePartyResolver) {
    let resolver: Arc<dyn TracePartyResolver> = Arc::new(resolver);
    app.provide(resolver);
}

/// The app's [`TracePartyResolver`], or [`NoTraceParties`] when none is
/// provided.
pub fn trace_party_resolver(state: &AppState) -> Arc<dyn TracePartyResolver> {
    match state.provided::<Arc<dyn TracePartyResolver>>() {
        Some(resolver) => Arc::clone(&resolver),
        None => Arc::new(NoTraceParties),
    }
}

/// One line of a recall's trace: who got the goods (wl-inventory §3 *Recall*,
/// affected line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracedHolder {
    /// The line itself, `inventory.recall_line:7` (the subject of its
    /// notices).
    pub line: Subject,
    /// The visit, order, job card, dispatch or work order that used the lot;
    /// `None` for a count of walk-in sales.
    pub subject: Option<Subject>,
    /// The patient, customer or other party to tell; `None` when the vertical
    /// does not know (or for walk-in sales).
    pub party: Option<PartyId>,
    /// The lot, `inventory.lot:4`.
    pub lot: Option<Subject>,
    /// Where it was used: the location it left, `inventory.location:2`.
    pub location: Option<Subject>,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The day it was used (Asia/Dubai).
    pub on: Option<NaiveDate>,
    /// `confirmed`, `uncertain`, `possible`, `dispatch` or `walk_in`.
    pub kind: String,
    /// How sure the trace is.
    pub mark: TraceMark,
    /// The sales counted, for `walk_in` (else 1).
    pub sales: i64,
    /// The delivery status of a dispatch.
    pub delivery_status: Option<String>,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
}

/// Where a lot of a recall is now: the stock the recall's holds block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracedPlace {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The location it is at (the in-transit one included).
    pub location: Subject,
    /// The quantity held there, in thousandths.
    pub quantity_milli: i64,
}

/// A recall as the verticals list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracedRecall {
    /// The recall, `inventory.recall:2`.
    pub recall: Subject,
    /// The number, `RCL-2026-0001`.
    pub number: String,
    /// `open`, `traced` or `closed`.
    pub status: String,
    /// Why it was opened.
    pub reason: String,
    /// The day it was opened (Asia/Dubai).
    pub opened_on: Option<NaiveDate>,
}

/// The forward trace of a recall, for other modules (wl-inventory §6
/// `LotTrace`): the clinics vertical reads the patients to call back, 06 the
/// customers. Without the inventory module [`lot_trace`] gives [`NoLotTrace`],
/// which knows nothing.
pub trait LotTrace: Send + Sync + 'static {
    /// The affected lines of `recall` (`inventory.recall:2`): subject, party,
    /// lot, quantity and notice status of each.
    fn holders<'a>(
        &'a self,
        _state: &'a AppState,
        _recall: &'a Subject,
    ) -> TraceFuture<'a, Vec<TracedHolder>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Where each lot of `recall` is, as its holds block it.
    fn places<'a>(
        &'a self,
        _state: &'a AppState,
        _recall: &'a Subject,
    ) -> TraceFuture<'a, Vec<TracedPlace>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    /// A vertical that sends its own notice (05: a neutral text with no
    /// product) tells the recall which line it told: `sent` marks the
    /// `pending` line `line` (`inventory.recall_line:7`) as sent, and `false`
    /// takes a `sent` line back to `pending` (the notice gave up). `by` is
    /// the user who sent it. Returns whether the line changed; a line that
    /// was waived, a closed recall or a line in the other state change
    /// nothing.
    fn set_notice<'a>(
        &'a self,
        _state: &'a AppState,
        _line: &'a Subject,
        _sent: bool,
        _by: Option<i64>,
    ) -> TraceFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
    /// The recalls, newest first (a vertical lists them beside its own
    /// customer view of the trace). The default has none.
    fn recalls<'a>(&'a self, _state: &'a AppState) -> TraceFuture<'a, Vec<TracedRecall>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// A vertical whose owner decided that the party of `line`
    /// (`inventory.recall_line:7`) need not be told (06: the public notice
    /// for sales without a known customer) waives it with `reason`. The
    /// vertical has checked who may decide. Returns whether the line
    /// changed; a line that was not `pending` or a closed recall change
    /// nothing.
    fn waive<'a>(
        &'a self,
        _state: &'a AppState,
        _line: &'a Subject,
        _reason: &'a str,
        _by: Option<i64>,
    ) -> TraceFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The trace when `inventory` is off: nothing is known.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLotTrace;

impl LotTrace for NoLotTrace {}

/// Makes `trace` the app's [`LotTrace`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_lot_trace(app: &mut Registry, trace: impl LotTrace) {
    let trace: Arc<dyn LotTrace> = Arc::new(trace);
    app.provide(trace);
}

/// The app's [`LotTrace`], or [`NoLotTrace`] when none is provided.
pub fn lot_trace(state: &AppState) -> Arc<dyn LotTrace> {
    match state.provided::<Arc<dyn LotTrace>>() {
        Some(trace) => Arc::clone(&trace),
        None => Arc::new(NoLotTrace),
    }
}

/// A recall was traced (wl-inventory §6 `RecallTraced`): the affected lines
/// exist and the lots made from the recalled ones are in its scope and held.
/// A re-run emits it again with every line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallTraced {
    /// The recall, `inventory.recall:2`.
    pub recall: Subject,
    /// Its number, `RCL-2026-0001`.
    pub number: String,
    /// Every lot in its scope now, the output lots included.
    pub lots: Vec<Subject>,
    /// The affected lines.
    pub lines: Vec<TracedHolder>,
}

impl Event for RecallTraced {}

/// A recall was closed (wl-inventory §6 `RecallClosed`): every lot has an
/// outcome and every affected party was told or waived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallClosed {
    /// The recall, `inventory.recall:2`.
    pub recall: Subject,
    /// Its number, `RCL-2026-0001`.
    pub number: String,
    /// The lots in its scope.
    pub lots: Vec<Subject>,
    /// The affected lines with their notice status.
    pub lines: Vec<TracedHolder>,
    /// What was decided for each held line.
    pub outcomes: Vec<HeldOutcome>,
}

impl Event for RecallClosed {}

/// One line of a dispatch, as the dispatch events carry it (wl-inventory §6).
/// Quantities are thousandths of the item's base unit, value is fils.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchedLine {
    /// The dispatch line, `inventory.dispatch_line:4`.
    pub line: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot the line was picked from, `inventory.lot:2`.
    pub lot: Option<Subject>,
    /// The line of the source it fulfils (a sales order line), when it has one.
    pub source_line: Option<Subject>,
    /// The quantity sent.
    pub quantity_milli: i64,
    /// The quantity the recipient accepted; 0 until the proof of delivery.
    pub delivered_milli: i64,
    /// The quantity the recipient refused; it is *in return* until it is
    /// received back into quarantine.
    pub refused_milli: i64,
    /// The refusal reason's code, when something was refused.
    pub refusal_reason: Option<String>,
    /// The value of the quantity sent (the cost it left stock at), in fils.
    pub value: i64,
}

/// A dispatch left (`dispatch`, `DispatchSent`): `wl-delivery` creates the
/// delivery for an own-delivery hand-over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchSent {
    /// The dispatch, `inventory.dispatch:3`.
    pub dispatch: Subject,
    /// The record it fulfils (a sales order).
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// `own_delivery`, `courier` or `collection`.
    pub handover: String,
    /// The delivery `wl-delivery` was asked for, for an own delivery.
    pub delivery: Option<i64>,
    /// The courier's tracking number.
    pub tracking_number: Option<String>,
    /// The lines.
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchSent {}

/// A dispatch was confirmed delivered (`confirm_delivery`,
/// `DispatchDelivered`): the source fulfils and bills the delivered quantity
/// only; the refused quantity is *in return*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchDelivered {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// The lines with the delivered and the refused quantity.
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchDelivered {}

/// A dispatch was lost (`mark_lost`, `DispatchLost`) and its value is held as
/// a loss while the claim runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchLost {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// Whether the dispatch was `delivered` (under review) when it was lost:
    /// the source then reverses its fulfilment.
    pub was_delivered: bool,
    /// The value held as a loss, in fils.
    pub value: i64,
    /// The lines (delivered quantity 0 when the dispatch had been delivered).
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchLost {}

/// A delivered dispatch was reopened because its delivery was reversed
/// (`reverse`, `DispatchReversed`): the source reverses its fulfilment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchReversed {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// The lines (delivered quantity 0; the refused quantity is unchanged).
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchReversed {}

/// A dispatch came back (`return`, `DispatchReturned`): refused or returned
/// whole. Its goods are in quarantine and a complaint ticket was opened; the
/// source keeps its reservation request and tells the rep and the customer
/// (wl-sales `return_delivery`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchReturned {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// Why it came back.
    pub reason: String,
    /// The complaint ticket opened by the return, when ticketing is on.
    #[serde(default)]
    pub complaint: Option<Subject>,
    /// The lines.
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchReturned {}

/// A perishable dispatch was destroyed after a failed attempt (`discard`,
/// `DispatchDiscarded`): its value went to wastage and no stock moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchDiscarded {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// The reason the driver gave.
    pub reason: String,
    /// The photo of the discarded goods.
    pub photo: Option<String>,
    /// The value posted as wastage, in fils.
    pub value: i64,
    /// The lines.
    pub lines: Vec<DispatchedLine>,
}

impl Event for DispatchDiscarded {}

/// Goods of a dispatch that came back and were booked into quarantine: the
/// whole quantity of a returned dispatch, or the refused quantity received
/// back at run close (`DeliveryRefusedReceived`, or by hand for a courier).
/// The vertical inspects each lot (07 `MFG.RUL.10`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchGoodsQuarantined {
    /// The dispatch.
    pub dispatch: Subject,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// Whether the whole dispatch came back (`return`); false for refused
    /// lines of a delivered dispatch.
    pub returned: bool,
    /// The complaint ticket of a returned dispatch, when there is one.
    #[serde(default)]
    pub complaint: Option<Subject>,
    /// What was booked: one entry per line.
    pub lines: Vec<QuarantinedLine>,
}

impl Event for DispatchGoodsQuarantined {}

/// The quantity of one dispatch line booked into quarantine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantinedLine {
    /// The dispatch line, `inventory.dispatch_line:4`.
    pub line: Subject,
    /// The item.
    pub item: Subject,
    /// The lot the goods were picked from (and are held as in quarantine).
    pub lot: Option<Subject>,
    /// The line of the source it fulfils (a sales order line).
    pub source_line: Option<Subject>,
    /// The quantity booked, in thousandths.
    pub quantity_milli: i64,
    /// The refusal or return reason.
    pub reason: Option<String>,
}

/// A dispatch as the verticals read it (wl-inventory §6 `DispatchDirectory`):
/// who it goes to, where it stands and what is on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchFacts {
    /// The dispatch, `inventory.dispatch:3`.
    pub dispatch: Subject,
    /// The number, `DSP-2026-0001`.
    pub number: String,
    /// The state: `draft`, `picked`, `dispatched`, `delivered`, `returned`,
    /// `discarded`, `lost` or `cancelled`.
    pub status: String,
    /// The record it fulfils (a sales order).
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<PartyId>,
    /// The day it left (Asia/Dubai); `None` until `dispatch`.
    pub dispatched_on: Option<NaiveDate>,
    /// The lines.
    pub lines: Vec<DispatchedLine>,
}

/// The dispatches, for the verticals that guard or follow them: the item
/// flags at `pick` and `dispatch` (07 `MFG.ACL.11`), the export evidence
/// (`MFG.RUL.08`) and the recall trace. `wl-inventory` provides it; without
/// it [`dispatch_directory`] gives [`NoDispatchDirectory`], which knows none.
pub trait DispatchDirectory: Send + Sync + 'static {
    /// The dispatch `dispatch` (`inventory.dispatch:3`); `None` when unknown.
    fn facts<'a>(
        &'a self,
        _state: &'a AppState,
        _dispatch: &'a Subject,
    ) -> TraceFuture<'a, Option<DispatchFacts>> {
        Box::pin(async { Ok(None) })
    }

    /// The dispatches made for `source` (a sales order), oldest first, in
    /// every state. A source reads how far its goods are (picked, out for
    /// delivery, delivered, lost). The default knows none.
    fn of_source<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
    ) -> TraceFuture<'a, Vec<DispatchFacts>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// The dispatches that left on or before `day` and have not been
    /// returned, cancelled or discarded, oldest first.
    fn left_by<'a>(
        &'a self,
        _state: &'a AppState,
        _day: NaiveDate,
    ) -> TraceFuture<'a, Vec<DispatchFacts>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The directory when nothing provides one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDispatchDirectory;

impl DispatchDirectory for NoDispatchDirectory {}

/// Makes `directory` the app's [`DispatchDirectory`].
pub fn provide_dispatch_directory(app: &mut Registry, directory: impl DispatchDirectory) {
    let directory: Arc<dyn DispatchDirectory> = Arc::new(directory);
    app.provide(directory);
}

/// The app's [`DispatchDirectory`], or [`NoDispatchDirectory`] when none is
/// provided.
pub fn dispatch_directory(state: &AppState) -> Arc<dyn DispatchDirectory> {
    match state.provided::<Arc<dyn DispatchDirectory>>() {
        Some(directory) => Arc::clone(&directory),
        None => Arc::new(NoDispatchDirectory),
    }
}

/// The item-flag guard of a dispatch (wl-inventory §4 *Dispatch* rows `pick`
/// and `dispatch`, STK.DSP.01; 07 `MFG.ACL.11`): the vertical says why the
/// goods on a dispatch may not be picked or may not leave, with no override.
/// The default blocks nothing. The refusal is a 422 with the words it gives;
/// the dispatch can still be cancelled.
pub trait DispatchGuard: Send + Sync + 'static {
    /// Why `dispatch` may not take the step `guarded` (`dispatch.pick` or
    /// `dispatch.dispatch`) on `on` (Asia/Dubai); `None` when it may.
    fn blocked<'a>(
        &'a self,
        _state: &'a AppState,
        _dispatch: &'a DispatchFacts,
        _guarded: &'a str,
        _on: NaiveDate,
    ) -> TraceFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

/// The guard when no vertical provides one: nothing is blocked.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDispatchGuard;

impl DispatchGuard for NoDispatchGuard {}

/// Makes `guard` the app's [`DispatchGuard`].
pub fn provide_dispatch_guard(app: &mut Registry, guard: impl DispatchGuard) {
    let guard: Arc<dyn DispatchGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's [`DispatchGuard`], or [`NoDispatchGuard`] when none is provided.
pub fn dispatch_guard(state: &AppState) -> Arc<dyn DispatchGuard> {
    match state.provided::<Arc<dyn DispatchGuard>>() {
        Some(guard) => Arc::clone(&guard),
        None => Arc::new(NoDispatchGuard),
    }
}

/// A production batch's use of one ingredient lot (wl-inventory §3 lot
/// genealogy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchInput {
    /// The ingredient lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The quantity of it that went into the batch, in thousandths.
    pub quantity_milli: i64,
}

/// A production batch finishes: the ingredient lots are consumed and the
/// output lot is made (06 F&B §7.1 *Production batch* `finish`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProduceBatch {
    /// The item that is made, `inventory.item:9`.
    pub item: Subject,
    /// Where the ingredients are taken from and the output arrives,
    /// `inventory.location:2`.
    pub location: Subject,
    /// The quantity made (the actual yield), in thousandths.
    pub quantity_milli: i64,
    /// The number of the output lot.
    pub lot_number: String,
    /// The last day the output can be used.
    pub expiry: NaiveDate,
    /// The batch: the source of every movement, so a recall trace follows the
    /// ingredient lots to the output lot.
    pub source: Subject,
    /// The ingredient lots used, with the quantity of each.
    pub inputs: Vec<BatchInput>,
    /// The user who finishes the batch.
    pub by: Option<i64>,
}

/// What a finished batch made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Produced {
    /// The output lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The reason of the hold an ingredient lot was on when the batch
    /// finished (the caller holds the output lot with it), else `None`.
    pub held_reason: Option<String>,
}

/// A lot as a vertical reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotInfo {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot number.
    pub number: String,
    /// The last day it can be used.
    pub expiry_date: Option<NaiveDate>,
    /// `quarantine`, `released`, `on_hold`, `expired`, `rejected` or
    /// `depleted`.
    pub status: String,
    /// Why it is on hold, while it is.
    pub held_reason: Option<String>,
    /// What is left of it in all locations, in thousandths.
    pub left_milli: i64,
    /// What is left, at the item's average cost, in fils.
    pub value: i64,
}

/// Quantity and value of the wastage posted against a lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Wasted {
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// Its value at the cost it was posted at, in fils.
    pub value: i64,
    /// How many wastage movements there are.
    pub movements: i64,
}

/// The stock side of a production batch (06 F&B §7.1 *Production batch*,
/// wl-inventory §3 *Lot*). Without the inventory module [`batch_stock`] gives
/// [`NoBatchStock`], which makes nothing.
pub trait BatchStock: Send + Sync + 'static {
    /// The lot, with its status and what is left.
    fn lot_info<'a>(
        &'a self,
        _state: &'a AppState,
        _lot: &'a Subject,
    ) -> StockFuture<'a, Option<LotInfo>> {
        Box::pin(async { Ok(None) })
    }

    /// Consumes the ingredient lots (a lot on hold is consumed too: the food
    /// was used) and makes the output lot with its expiry and the `produce`
    /// movement. `None` when the module is off.
    fn produce<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ProduceBatch,
    ) -> StockFuture<'a, Option<Produced>> {
        Box::pin(async { Ok(None) })
    }

    /// Puts the whole lot on hold with `reason` (`recall`, `quality`,
    /// `temperature`...), caused by `reference`; `LotHeld` follows. Returns
    /// whether a hold was placed.
    fn hold_lot<'a>(
        &'a self,
        _state: &'a AppState,
        _lot: &'a Subject,
        _reason: &'a str,
        _reference: &'a Subject,
    ) -> StockFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Posts what is left of a lot that is not on hold as wastage with
    /// `reason`, as `by`. Returns the quantity and value posted.
    fn waste_lot<'a>(
        &'a self,
        _state: &'a AppState,
        _lot: &'a Subject,
        _reason: &'a str,
        _by: Option<i64>,
    ) -> StockFuture<'a, Wasted> {
        Box::pin(async { Ok(Wasted::default()) })
    }

    /// The wastage already posted against the lot (the discard of a hold
    /// posts it): the batch's wastage entry links it and posts no second
    /// movement.
    fn wasted_of_lot<'a>(
        &'a self,
        _state: &'a AppState,
        _lot: &'a Subject,
    ) -> StockFuture<'a, Wasted> {
        Box::pin(async { Ok(Wasted::default()) })
    }
}

/// The batch stock when `inventory` is off: nothing is made.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBatchStock;

impl BatchStock for NoBatchStock {}

/// Makes `stock` the app's [`BatchStock`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_batch_stock(app: &mut Registry, stock: impl BatchStock) {
    let stock: Arc<dyn BatchStock> = Arc::new(stock);
    app.provide(stock);
}

/// The app's [`BatchStock`], or [`NoBatchStock`] when none is provided.
pub fn batch_stock(state: &AppState) -> Arc<dyn BatchStock> {
    match state.provided::<Arc<dyn BatchStock>>() {
        Some(stock) => Arc::clone(&stock),
        None => Arc::new(NoBatchStock),
    }
}
