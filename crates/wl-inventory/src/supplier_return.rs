//! The Return to supplier (wl-inventory §3, §4 *Return to supplier*): goods
//! sent back to a supplier for credit or replacement. A posted goods receipt
//! makes one from its rejected lines (§5.6): one for the goods refused at the
//! door (the goods receipt is the collection proof) and one for the goods kept
//! for collection, both `draft`; the door-refused one is dispatched at once
//! by the system, and a return of stock that failed is made with
//! [`create_from_stock`]. `dispatch` is in [`crate::return_dispatch`];
//! `credit` and `cancel` are in [`crate::return_credit`].

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::SUPPLIER_RETURN_SUBJECT_TYPE;

use crate::item::Item;
use crate::purchase_order::PurchaseOrderLine;
use crate::receipt_post::SUPPLIER_RETURN_SERIES;
use crate::texts::said;

/// Where a return to supplier stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnStatus {
    /// Lines being prepared, or rejected goods waiting for collection.
    #[default]
    Draft,
    /// The goods left with the supplier or their driver.
    Dispatched,
    /// The supplier credited or replaced everything (terminal).
    Credited,
    /// Not sent; the goods are kept (terminal).
    Cancelled,
}

/// What happened to the rejected goods.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RejectedHandling {
    /// The supplier's driver takes the goods back at once.
    #[default]
    RefusedAtDoor,
    /// The goods wait aside, not in stock, until the supplier collects.
    KeptForCollection,
}

/// What the user wants for rejected goods.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WantedOutcome {
    /// A replacement on the same order: the rejected quantity stays
    /// outstanding.
    #[default]
    Replace,
    /// A credit: the supplier issues a credit note.
    Credit,
}

/// Where the goods of a line came from.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnOrigin {
    /// Rejected at receipt: they never entered stock.
    #[default]
    RejectedAtReceipt,
    /// Stock that failed later.
    FromStock,
}

/// Goods sent back to a supplier.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_supplier_returns",
    state_machine,
    unique(number),
    index(source_type, source_id),
    index(status)
)]
pub struct SupplierReturn {
    /// The primary key.
    pub id: i64,
    /// The number from the `RTS-` series.
    pub number: Option<String>,
    /// The supplier.
    pub supplier_party_id: i64,
    /// The type of the record the goods came from (the goods receipt).
    pub source_type: String,
    /// The id of the record the goods came from.
    pub source_id: i64,
    /// Refused at the door, or kept for collection.
    pub handling: RejectedHandling,
    /// The collection proof: the goods receipt for goods refused at the door.
    pub collection_proof: Option<String>,
    /// The location the stock lines leave from.
    pub location_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: ReturnStatus,
    /// The user who dispatched it (who asked, when it waited for approval);
    /// `None` for the system.
    pub dispatched_by: Option<i64>,
    /// When it was dispatched.
    pub dispatched_at: Option<DateTime>,
    /// The return note, a PDF document.
    pub return_note: Option<String>,
    /// Why a draft return was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who cancelled it; `None` for the system.
    pub cancelled_by: Option<i64>,
    /// When it was cancelled.
    pub cancelled_at: Option<DateTime>,
    /// What became of the goods kept for collection when it was cancelled:
    /// `received_again` or `discarded`.
    pub kept_goods: Option<String>,
    /// How it was credited: `debit_note`, `replacement`, `refund_received`
    /// or `supplier_refused`.
    pub credit_basis: Option<String>,
    /// Why it was credited by hand.
    pub credit_reason: Option<String>,
    /// The user who credited it by hand.
    pub credited_by: Option<i64>,
    /// When it was credited.
    pub credited_at: Option<DateTime>,
    /// The debit note that completed the cover, in wl-invoicing.
    pub debit_note_id: Option<i64>,
    /// The value posted as wastage when the supplier refused the credit, in
    /// fils.
    pub wastage_fils: i64,
    /// The day the last `STK.RUL.08` notice went out.
    pub last_chased_on: Option<renox::chrono::NaiveDate>,
    /// How many `STK.RUL.08` notices went out.
    pub chase_count: i64,
    /// The user who made it; `None` for the system.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierReturn {
    /// The return as a [`Subject`]: `inventory.supplier_return:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUPPLIER_RETURN_SUBJECT_TYPE, self.id)
    }

    /// The record the goods came from.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }
}

/// One line of a return. Quantities are thousandths of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_supplier_return_lines",
    unique(supplier_return_id, position)
)]
pub struct SupplierReturnLine {
    /// The primary key.
    pub id: i64,
    /// The return.
    pub supplier_return_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The item.
    pub item_id: i64,
    /// The lot, if any.
    pub lot_id: Option<i64>,
    /// The quantity sent back.
    pub quantity_milli: i64,
    /// The order's price of one base unit.
    pub unit_cost: i64,
    /// Why: the rejection reason.
    pub reason: String,
    /// Replacement or credit.
    pub wanted_outcome: WantedOutcome,
    /// The purchase order line.
    pub po_line_id: Option<i64>,
    /// Rejected at receipt, or from stock.
    pub origin: ReturnOrigin,
    /// The goods receipt line the goods were rejected on.
    pub receipt_line_id: Option<i64>,
    /// (Source unit cost - average cost) x quantity in fils, set at
    /// `dispatch` for a line taken from stock: the purchase price variance.
    pub variance: i64,
    /// The quantity the line gave back to its order line, in thousandths.
    pub restored_milli: i64,
    /// What a supplier debit note (credit line) or a replacement receipt
    /// (replace line) has covered of the line, in thousandths.
    pub covered_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line of a return to make from stock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewReturnLine {
    /// The item.
    pub item_id: i64,
    /// The lot, if any.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// Why the goods go back.
    pub reason: String,
    /// Credit or replacement.
    pub wanted_outcome: WantedOutcome,
    /// The purchase order line a replacement is counted on.
    pub po_line_id: Option<i64>,
    /// The cost of one base unit the goods are valued at: the source
    /// receipt's when known. The item's average cost when `None`.
    pub unit_cost: Option<i64>,
}

impl NewReturnLine {
    /// `quantity_milli` of `item_id` going back for `reason`.
    pub fn of(item_id: i64, quantity_milli: i64, reason: &str) -> Self {
        NewReturnLine {
            item_id,
            quantity_milli,
            reason: reason.to_owned(),
            ..NewReturnLine::default()
        }
    }

    /// Of lot `lot_id`.
    pub fn lot(mut self, lot_id: i64) -> Self {
        self.lot_id = Some(lot_id);
        self
    }

    /// Credited, not replaced.
    pub fn credit(mut self) -> Self {
        self.wanted_outcome = WantedOutcome::Credit;
        self
    }

    /// Replaced on `po_line_id`.
    pub fn replacing_on(mut self, po_line_id: i64) -> Self {
        self.wanted_outcome = WantedOutcome::Replace;
        self.po_line_id = Some(po_line_id);
        self
    }

    /// Valued at `unit_cost` fils a unit (the source receipt's cost).
    pub fn at_cost(mut self, unit_cost: i64) -> Self {
        self.unit_cost = Some(unit_cost);
        self
    }
}

/// A return to make from stock, as a `draft`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReturn {
    /// The supplier.
    pub supplier: wl_kernel::PartyId,
    /// The record the goods came from: a stock hold, a recall, an NCR...
    pub source: Subject,
    /// The location the goods leave from (the quarantine or source location).
    pub location_id: i64,
    /// The lines.
    pub lines: Vec<NewReturnLine>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Records `new` as a `draft` Return to supplier by user `by`, its lines
/// taken from stock at the location, kept for collection.
///
/// # Errors
///
/// 422 for a return without lines, a line without quantity or reason, or an
/// unknown item.
pub async fn create_from_stock(db: &Db, by: Option<i64>, new: NewReturn) -> Result<SupplierReturn> {
    if new.lines.is_empty() {
        return Err(refused(said("wl_inventory.return.no_lines", &[])));
    }
    let mut rows = Vec::new();
    for (position, line) in (1..).zip(&new.lines) {
        let position_text = position.to_string();
        if line.quantity_milli <= 0 || line.reason.trim().is_empty() {
            return Err(refused(said(
                "wl_inventory.return.bad_line",
                &[("position", &position_text)],
            )));
        }
        let Some(item) = Item::find(db, line.item_id).await? else {
            return Err(refused(said(
                "wl_inventory.receipt.unknown_item",
                &[("position", &position_text)],
            )));
        };
        let po_price = match line.po_line_id {
            Some(id) => PurchaseOrderLine::find(db, id).await?.map(|l| l.unit_price),
            None => None,
        };
        rows.push(SupplierReturnLine {
            position,
            item_id: item.id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
            unit_cost: line.unit_cost.or(po_price).unwrap_or(item.average_cost),
            reason: line.reason.trim().to_owned(),
            wanted_outcome: line.wanted_outcome,
            po_line_id: line.po_line_id,
            origin: ReturnOrigin::FromStock,
            ..SupplierReturnLine::default()
        });
    }
    let mut ret = SupplierReturn {
        supplier_party_id: new.supplier.0,
        source_type: new.source.subject_type.clone(),
        source_id: new.source.subject_id,
        handling: RejectedHandling::KeptForCollection,
        location_id: Some(new.location_id),
        status: ReturnStatus::Draft,
        created_by: by,
        ..SupplierReturn::default()
    };
    let mut tx = db.begin().await?;
    ret.number = Some(
        SUPPLIER_RETURN_SERIES
            .next(&mut tx, None, None, today())
            .await?,
    );
    ret.insert(&mut tx).await?;
    for row in &mut rows {
        row.supplier_return_id = ret.id;
        row.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(ret)
}

/// What draft returns hold aside of `item_id` at `location_id` (of `lot_id`
/// when given), in thousandths: the stock lines of returns not sent yet.
/// Cancelling a draft return releases them.
pub async fn reserved_milli(
    db: &Db,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    let drafts = SupplierReturn::query()
        .where_(SupplierReturn::STATUS.eq(ReturnStatus::Draft))
        .where_(SupplierReturn::LOCATION_ID.eq(Some(location_id)))
        .get(db)
        .await?;
    let mut reserved = 0;
    for draft in drafts {
        reserved += lines_of(db, draft.id)
            .await?
            .iter()
            .filter(|line| line.origin == ReturnOrigin::FromStock && line.item_id == item_id)
            .filter(|line| lot_id.is_none() || line.lot_id == lot_id)
            .map(|line| line.quantity_milli)
            .sum::<i64>();
    }
    Ok(reserved)
}

/// The lines of return `supplier_return_id`, in order, read in `tx`.
pub async fn lines_in(
    tx: &mut Transaction,
    supplier_return_id: i64,
) -> Result<Vec<SupplierReturnLine>> {
    SupplierReturnLine::query()
        .where_(SupplierReturnLine::SUPPLIER_RETURN_ID.eq(supplier_return_id))
        .order_by(SupplierReturnLine::POSITION)
        .get(&mut *tx)
        .await
}

/// The lines of return `supplier_return_id`, in order.
pub async fn lines_of(db: &Db, supplier_return_id: i64) -> Result<Vec<SupplierReturnLine>> {
    SupplierReturnLine::query()
        .where_(SupplierReturnLine::SUPPLIER_RETURN_ID.eq(supplier_return_id))
        .order_by(SupplierReturnLine::POSITION)
        .get(db)
        .await
}

/// The returns made from `source`, oldest first.
pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<SupplierReturn>> {
    SupplierReturn::query()
        .where_(SupplierReturn::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(SupplierReturn::SOURCE_ID.eq(source.subject_id))
        .order_by(SupplierReturn::ID)
        .get(db)
        .await
}

/// The returns made from `source`, oldest first, read in `tx`.
pub async fn of_source_in(tx: &mut Transaction, source: &Subject) -> Result<Vec<SupplierReturn>> {
    SupplierReturn::query()
        .where_(SupplierReturn::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(SupplierReturn::SOURCE_ID.eq(source.subject_id))
        .order_by(SupplierReturn::ID)
        .get(&mut *tx)
        .await
}

impl StateMachine for SupplierReturn {
    type State = ReturnStatus;
    const COLUMN: renox::db::Col<SupplierReturn, ReturnStatus> = SupplierReturn::STATUS;

    fn machine() -> Machine<Self> {
        use ReturnStatus::{Cancelled, Credited, Dispatched, Draft};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Dispatched)
            .tone("info")
            .state(Credited)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        crate::return_credit::add_to(crate::return_dispatch::add_to(machine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_starts_as_a_draft_with_the_four_states() {
        let machine = SupplierReturn::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.states().len(), 4);
    }

    #[test]
    fn the_handling_and_the_outcome_are_stored_in_snake_case() {
        assert_eq!(RejectedHandling::RefusedAtDoor.as_str(), "refused_at_door");
        assert_eq!(
            RejectedHandling::KeptForCollection.as_str(),
            "kept_for_collection"
        );
        assert_eq!(WantedOutcome::default().as_str(), "replace");
        assert_eq!(ReturnOrigin::default().as_str(), "rejected_at_receipt");
    }
}
