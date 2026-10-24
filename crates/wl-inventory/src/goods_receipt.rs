//! The Goods receipt (wl-inventory §3, §4 *Goods receipt*): a delivery
//! against a purchase order, with the received, accepted and rejected
//! quantity of every line. Only the accepted quantity enters stock, the order
//! and the bill match. The machine and `post` are in [`crate::receipt_post`].
//!
//! ```text
//! [*] --create--> draft --post--> posted --reverse--> reversed
//! ```
//!
//! `reverse` is in [`crate::receipt_reverse`]. A receipt is recorded as a
//! draft ([`create_draft`]), its lines are filled in as the storekeeper counts
//! ([`update_line`]), and posted by a holder of `goods_receipts.post` at the
//! location.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::GOODS_RECEIPT_SUBJECT_TYPE;
use wl_kernel::{PartyId, Subject};

use crate::location::{Location, scope_of};
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine};
use crate::supplier_return::{RejectedHandling, WantedOutcome};
use crate::texts::said;

/// Where a goods receipt stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GoodsReceiptStatus {
    /// Quantities, lots and expiry being entered.
    #[default]
    Draft,
    /// Stock and cost updated (terminal until `reverse`).
    Posted,
    /// Cancelled by a reversing movement (terminal).
    Reversed,
}

/// A delivery against a purchase order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_goods_receipts",
    state_machine,
    unique(number),
    index(status),
    index(purchase_order_id)
)]
pub struct GoodsReceipt {
    /// The primary key.
    pub id: i64,
    /// The number from the `GRN-` series, set at `post`.
    pub number: Option<String>,
    /// The purchase order; `None` for a free replacement.
    pub purchase_order_id: Option<i64>,
    /// The dispatched Return to supplier a free replacement is received
    /// against; `None` for a receipt on a purchase order.
    pub supplier_return_id: Option<i64>,
    /// The supplier.
    pub supplier_party_id: i64,
    /// The location the goods arrive at.
    pub location_id: i64,
    /// The type of the location's scope subject, copied.
    pub scope_type: Option<String>,
    /// The id of the location's scope subject, copied.
    pub scope_id: Option<i64>,
    /// The supplier's delivery note number.
    pub delivery_note: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: GoodsReceiptStatus,
    /// The user who recorded it.
    pub created_by: Option<i64>,
    /// The user who posted it (who asked, when it waited for approval).
    pub posted_by: Option<i64>,
    /// When it was posted.
    pub posted_at: Option<DateTime>,
    /// When the goods arrived, when that is not the posting: the business
    /// time of a late entry, or the moment posting was asked for (the supplier
    /// score counts the day the goods arrived, not the day an approval came).
    pub arrived_at: Option<DateTime>,
    /// Whether the receipt was entered late, after an outage.
    pub late_entry: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl GoodsReceipt {
    /// The receipt as a [`Subject`]: `inventory.goods_receipt:5`.
    pub fn subject(&self) -> Subject {
        Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, self.id)
    }

    /// The supplier.
    pub fn supplier(&self) -> PartyId {
        PartyId(self.supplier_party_id)
    }

    /// The permission scope of the receiving location: who may post it, and
    /// who approves an over-receipt.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }

    /// The purchase order as a [`Subject`], if any.
    pub fn purchase_order(&self) -> Option<Subject> {
        self.purchase_order_id
            .map(|id| Subject::new(wl_kernel::inventory::PURCHASE_ORDER_SUBJECT_TYPE, id))
    }
}

/// One line of a goods receipt. Quantities are thousandths of the item's base
/// unit, money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_goods_receipt_lines",
    unique(goods_receipt_id, position)
)]
pub struct GoodsReceiptLine {
    /// The primary key.
    pub id: i64,
    /// The receipt.
    pub goods_receipt_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The purchase order line it is received on.
    pub po_line_id: Option<i64>,
    /// The return line a free replacement replaces.
    pub return_line_id: Option<i64>,
    /// The item.
    pub item_id: i64,
    /// What arrived.
    pub received_qty_milli: i64,
    /// What entered stock and counts on the order.
    pub accepted_qty_milli: i64,
    /// What was refused; never in stock.
    pub rejected_qty_milli: i64,
    /// Why: `temperature`, `quality`... required for a rejected quantity.
    pub rejection_reason: Option<String>,
    /// Refused at the door, or kept for collection.
    pub rejected_handling: Option<RejectedHandling>,
    /// Replacement (the default) or credit for the rejected quantity.
    pub wanted_outcome: WantedOutcome,
    /// The lot number, for a lot-tracked item.
    pub lot_number: Option<String>,
    /// The expiry date, for a lot-tracked item.
    pub expiry_date: Option<NaiveDate>,
    /// A reading or the reference of a photo, when the reason needs one.
    pub evidence: Option<String>,
    /// The order's price of one base unit.
    pub unit_cost: i64,
    /// The lot made at `post` for an accepted lot-tracked line.
    pub lot_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What the storekeeper enters for one line.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DraftLine {
    /// The purchase order line it is received on.
    pub po_line_id: Option<i64>,
    /// The return line it replaces, for a receipt against a return.
    pub return_line_id: Option<i64>,
    /// What arrived, in thousandths of the base unit.
    pub received_qty_milli: i64,
    /// What is accepted.
    pub accepted_qty_milli: i64,
    /// What is rejected.
    pub rejected_qty_milli: i64,
    /// Why the rejected quantity was refused.
    pub rejection_reason: Option<String>,
    /// Refused at the door, or kept for collection.
    pub rejected_handling: Option<RejectedHandling>,
    /// Replacement or credit.
    pub wanted_outcome: WantedOutcome,
    /// The lot number.
    pub lot_number: Option<String>,
    /// The expiry date.
    pub expiry_date: Option<NaiveDate>,
    /// A reading or a photo reference.
    pub evidence: Option<String>,
}

impl DraftLine {
    /// A line received on `po_line_id`: `received`, of which `accepted` is
    /// accepted and the rest rejected (thousandths).
    pub fn on(po_line_id: i64, received: i64, accepted: i64) -> Self {
        DraftLine {
            po_line_id: Some(po_line_id),
            received_qty_milli: received,
            accepted_qty_milli: accepted,
            rejected_qty_milli: received - accepted,
            ..DraftLine::default()
        }
    }

    /// A line replacing return line `return_line_id`: `received`, of which
    /// `accepted` is accepted and the rest rejected (thousandths).
    pub fn replacing(return_line_id: i64, received: i64, accepted: i64) -> Self {
        DraftLine {
            return_line_id: Some(return_line_id),
            received_qty_milli: received,
            accepted_qty_milli: accepted,
            rejected_qty_milli: received - accepted,
            ..DraftLine::default()
        }
    }

    /// With a reason for the rejected quantity, and what happened to the goods.
    pub fn rejected_for(mut self, reason: &str, handling: RejectedHandling) -> Self {
        self.rejection_reason = Some(reason.to_owned());
        self.rejected_handling = Some(handling);
        self
    }

    /// With a reading or a photo reference.
    pub fn evidence(mut self, evidence: &str) -> Self {
        self.evidence = Some(evidence.to_owned());
        self
    }

    /// With a lot and its expiry date.
    pub fn lot(mut self, number: &str, expiry: NaiveDate) -> Self {
        self.lot_number = Some(number.to_owned());
        self.expiry_date = Some(expiry);
        self
    }

    /// Credited, not replaced, when rejected.
    pub fn credit(mut self) -> Self {
        self.wanted_outcome = WantedOutcome::Credit;
        self
    }
}

/// A receipt to record as a draft.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DraftReceipt {
    /// The purchase order.
    pub purchase_order_id: i64,
    /// The location the goods arrive at.
    pub location_id: i64,
    /// The supplier's delivery note number.
    pub delivery_note: Option<String>,
    /// The lines.
    pub lines: Vec<DraftLine>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Records `draft` as a `draft` goods receipt by user `by`, with its lines
/// at the order's prices. The scope is the location's.
///
/// # Errors
///
/// 422 for an unknown order or location, or a line that names no line of the
/// order or has no item.
pub async fn create_draft(db: &Db, by: Option<i64>, draft: DraftReceipt) -> Result<GoodsReceipt> {
    let order = PurchaseOrder::find(db, draft.purchase_order_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.receipt.unknown_order", &[])))?;
    let location = Location::find(db, draft.location_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.receipt.unknown_location", &[])))?;
    let order_lines = crate::purchase_order::lines_of(db, order.id).await?;
    let mut lines = Vec::new();
    for (position, line) in (1..).zip(&draft.lines) {
        let po_line: Option<&PurchaseOrderLine> = line
            .po_line_id
            .and_then(|id| order_lines.iter().find(|l| l.id == id));
        let Some(po_line) = po_line else {
            return Err(refused(said(
                "wl_inventory.receipt.line_not_on_order",
                &[("position", &position.to_string())],
            )));
        };
        lines.push(line_of(0, position, po_line, line));
    }
    let mut receipt = GoodsReceipt {
        purchase_order_id: Some(order.id),
        supplier_party_id: order.supplier_party_id,
        location_id: location.id,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        delivery_note: draft
            .delivery_note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        status: GoodsReceiptStatus::Draft,
        created_by: by,
        ..GoodsReceipt::default()
    };
    let mut tx = db.begin().await?;
    receipt.insert(&mut tx).await?;
    for line in &mut lines {
        line.goods_receipt_id = receipt.id;
        line.insert(&mut tx).await?;
    }
    crate::search::index(&mut tx, GOODS_RECEIPT_SUBJECT_TYPE, receipt.id).await?;
    tx.commit().await?;
    Ok(receipt)
}

/// Records `lines` as a `draft` goods receipt against the dispatched Return
/// to supplier `return_id` by user `by`: a free replacement, at zero cost,
/// with no purchase order. Each line names the return line it replaces
/// ([`DraftLine::replacing`]).
///
/// # Errors
///
/// 422 for an unknown or not dispatched return or location, or a line that
/// names no line of the return.
pub async fn create_replacement_draft(
    db: &Db,
    by: Option<i64>,
    return_id: i64,
    location_id: i64,
    delivery_note: Option<String>,
    lines: Vec<DraftLine>,
) -> Result<GoodsReceipt> {
    let ret = crate::supplier_return::SupplierReturn::find(db, return_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.receipt.unknown_return", &[])))?;
    if ret.status != crate::supplier_return::ReturnStatus::Dispatched {
        return Err(refused(said(
            "wl_inventory.receipt.return_not_dispatched",
            &[],
        )));
    }
    let location = Location::find(db, location_id)
        .await?
        .ok_or_else(|| refused(said("wl_inventory.receipt.unknown_location", &[])))?;
    let return_lines = crate::supplier_return::lines_of(db, ret.id).await?;
    let mut rows = Vec::new();
    for (position, line) in (1..).zip(&lines) {
        let found = line
            .return_line_id
            .and_then(|id| return_lines.iter().find(|l| l.id == id));
        let Some(return_line) = found else {
            return Err(refused(said(
                "wl_inventory.receipt.line_not_on_return",
                &[("position", &position.to_string())],
            )));
        };
        rows.push(GoodsReceiptLine {
            goods_receipt_id: 0,
            position,
            po_line_id: None,
            return_line_id: Some(return_line.id),
            item_id: return_line.item_id,
            received_qty_milli: line.received_qty_milli,
            accepted_qty_milli: line.accepted_qty_milli,
            rejected_qty_milli: line.rejected_qty_milli,
            rejection_reason: clean(&line.rejection_reason),
            rejected_handling: line.rejected_handling,
            wanted_outcome: line.wanted_outcome,
            lot_number: clean(&line.lot_number),
            expiry_date: line.expiry_date,
            evidence: clean(&line.evidence),
            // A free replacement comes in at zero cost.
            unit_cost: 0,
            ..GoodsReceiptLine::default()
        });
    }
    let mut receipt = GoodsReceipt {
        purchase_order_id: None,
        supplier_return_id: Some(ret.id),
        supplier_party_id: ret.supplier_party_id,
        location_id: location.id,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        delivery_note: delivery_note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        status: GoodsReceiptStatus::Draft,
        created_by: by,
        ..GoodsReceipt::default()
    };
    let mut tx = db.begin().await?;
    receipt.insert(&mut tx).await?;
    for row in &mut rows {
        row.goods_receipt_id = receipt.id;
        row.insert(&mut tx).await?;
    }
    crate::search::index(&mut tx, GOODS_RECEIPT_SUBJECT_TYPE, receipt.id).await?;
    tx.commit().await?;
    Ok(receipt)
}

/// The stored line for `draft` on `po_line`.
fn line_of(
    receipt_id: i64,
    position: i64,
    po_line: &PurchaseOrderLine,
    draft: &DraftLine,
) -> GoodsReceiptLine {
    GoodsReceiptLine {
        goods_receipt_id: receipt_id,
        position,
        po_line_id: Some(po_line.id),
        item_id: po_line.item_id,
        received_qty_milli: draft.received_qty_milli,
        accepted_qty_milli: draft.accepted_qty_milli,
        rejected_qty_milli: draft.rejected_qty_milli,
        rejection_reason: clean(&draft.rejection_reason),
        rejected_handling: draft.rejected_handling,
        wanted_outcome: draft.wanted_outcome,
        lot_number: clean(&draft.lot_number),
        expiry_date: draft.expiry_date,
        evidence: clean(&draft.evidence),
        unit_cost: po_line.unit_price,
        ..GoodsReceiptLine::default()
    }
}

/// `text` trimmed; `None` when it is empty.
fn clean(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Replaces what is entered on line `line_id` of draft receipt
/// `receipt_id` with `draft` (the storekeeper counts again).
///
/// # Errors
///
/// 404 for an unknown line; 422 when the receipt is no longer a `draft`.
pub async fn update_line(
    db: &Db,
    receipt_id: i64,
    line_id: i64,
    draft: &DraftLine,
) -> Result<GoodsReceiptLine> {
    let receipt = GoodsReceipt::find_or_404(db, receipt_id).await?;
    if receipt.status != GoodsReceiptStatus::Draft {
        return Err(refused(said("wl_inventory.receipt.not_draft", &[])));
    }
    let mut line = GoodsReceiptLine::find_or_404(db, line_id).await?;
    if line.goods_receipt_id != receipt.id {
        return Err(Error::NotFound);
    }
    line.received_qty_milli = draft.received_qty_milli;
    line.accepted_qty_milli = draft.accepted_qty_milli;
    line.rejected_qty_milli = draft.rejected_qty_milli;
    line.rejection_reason = clean(&draft.rejection_reason);
    line.rejected_handling = draft.rejected_handling;
    line.wanted_outcome = draft.wanted_outcome;
    line.lot_number = clean(&draft.lot_number);
    line.expiry_date = draft.expiry_date;
    line.evidence = clean(&draft.evidence);
    line.save(db).await?;
    Ok(line)
}

/// The lines of receipt `receipt_id`, in order.
pub async fn lines_of(db: &Db, receipt_id: i64) -> Result<Vec<GoodsReceiptLine>> {
    GoodsReceiptLine::query()
        .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt_id))
        .order_by(GoodsReceiptLine::POSITION)
        .get(db)
        .await
}

/// Records the day the goods of the draft receipt `receipt_id` arrived when
/// it is not the day of the posting: the business time of an entry made late
/// after an outage (`late` sets the late-entry flag). The supplier score
/// counts this day (wl-inventory §3).
///
/// # Errors
///
/// 422 unless the receipt is a draft.
pub async fn record_arrival(
    db: &Db,
    receipt_id: i64,
    arrived_at: DateTime,
    late: bool,
) -> Result<()> {
    let receipt = GoodsReceipt::find_or_404(db, receipt_id).await?;
    if receipt.status != GoodsReceiptStatus::Draft {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            said("wl_inventory.receipt.not_draft", &[]),
        ));
    }
    GoodsReceipt::query()
        .where_(GoodsReceipt::ID.eq(receipt_id))
        .update(
            db,
            &[("arrived_at", &Some(arrived_at)), ("late_entry", &late)],
        )
        .await?;
    Ok(())
}

/// The subject type of the late entry of a goods receipt: the key of the
/// limits M and N (`PLT.ACL.10`).
pub const LATE_SUBJECT_TYPE: &str = "inventory.goods_receipt";

/// Registers the receipt with the kernel's late-entry check: the arrival day
/// of a draft receipt is its business time.
pub(crate) fn register_late_entry() {
    use wl_kernel::late_entries::LateEntryDef;
    LateEntryDef::of(LATE_SUBJECT_TYPE, |tx, new| {
        Box::pin(async move {
            let id = new.existing.ok_or(Error::NotFound)?;
            let receipt = GoodsReceipt::find(&mut *tx, id)
                .await?
                .ok_or(Error::NotFound)?;
            if receipt.status != GoodsReceiptStatus::Draft {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    said("wl_inventory.receipt.not_draft", &[]),
                ));
            }
            GoodsReceipt::query()
                .where_(GoodsReceipt::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("arrived_at", &Some(new.occurred_at)),
                        ("late_entry", &new.late),
                    ],
                )
                .await?;
            Ok(id)
        })
    })
    .business_time("arrived_at")
    .register();
}

/// Enters the arrival time of the draft receipt `receipt_id` through the
/// kernel's late-entry check with the limits of [`LATE_SUBJECT_TYPE`]: more
/// than M minutes before now it needs a `reason` and is flagged; more than N
/// hours before now it waits for a holder of `late_entries.approve`, and the
/// receipt keeps its old arrival time until then.
///
/// # Errors
///
/// 422 unless the receipt is a draft, when the entry is late and has no
/// reason, or when no approval policy is active for one that needs it.
pub async fn enter_arrival(
    state: &AppState,
    user_id: i64,
    receipt_id: i64,
    arrived_at: DateTime,
    reason: Option<&str>,
) -> Result<wl_kernel::late_entries::Entered> {
    let receipt = GoodsReceipt::find_or_404(&state.db, receipt_id).await?;
    if receipt.status != GoodsReceiptStatus::Draft {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            said("wl_inventory.receipt.not_draft", &[]),
        ));
    }
    let mut spec = wl_kernel::late_entries::EntrySpec::new(LATE_SUBJECT_TYPE, user_id, arrived_at)
        .existing(receipt_id)
        .parent(Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, receipt_id));
    if let Some(reason) = reason {
        spec = spec.reason(reason);
    }
    wl_kernel::late_entries::enter(state, spec).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_what_was_received_split_into_accepted_and_rejected() {
        let line = DraftLine::on(7, 100_000, 80_000)
            .rejected_for("temperature", RejectedHandling::RefusedAtDoor)
            .evidence("9 C");
        assert_eq!(line.po_line_id, Some(7));
        assert_eq!(line.rejected_qty_milli, 20_000);
        assert_eq!(line.rejection_reason.as_deref(), Some("temperature"));
        assert_eq!(line.wanted_outcome, WantedOutcome::Replace);
        assert_eq!(line.credit().wanted_outcome, WantedOutcome::Credit);
    }

    #[test]
    fn a_stored_line_takes_the_orders_item_and_price_and_trims_text() {
        let po_line = PurchaseOrderLine::new(3, 1, 9, 100_000, 1_200);
        let draft = DraftLine {
            lot_number: Some("  ".into()),
            evidence: Some(" 9 C ".into()),
            ..DraftLine::on(0, 100_000, 100_000)
        };
        let line = line_of(5, 1, &PurchaseOrderLine { id: 4, ..po_line }, &draft);
        assert_eq!(
            (line.item_id, line.unit_cost, line.po_line_id),
            (9, 1_200, Some(4))
        );
        assert_eq!(line.lot_number, None);
        assert_eq!(line.evidence.as_deref(), Some("9 C"));
    }

    #[test]
    fn a_receipt_has_the_scope_of_its_location() {
        let global = GoodsReceipt::default();
        assert_eq!(global.scope(), Scope::global());
        let site = GoodsReceipt {
            scope_type: Some("construction.project".into()),
            scope_id: Some(4),
            ..GoodsReceipt::default()
        };
        assert_eq!(site.scope(), Scope::new("construction.project", 4));
    }
}
