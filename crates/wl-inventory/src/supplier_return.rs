//! The Return to supplier (wl-inventory §3, §4 *Return to supplier*): goods
//! sent back to a supplier for credit or replacement. A posted goods receipt
//! makes one from its rejected lines (§5.6): one for the goods refused at the
//! door (the goods receipt is the collection proof) and one for the goods kept
//! for collection, both `draft`. The system `dispatch` of a door-refused
//! return, `credit` and `cancel` come with the issues that build them
//! (BL-STK-03, BL-STK-05): until then a door-refused return stays `draft`.
//! The machine declares the states.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::SUPPLIER_RETURN_SUBJECT_TYPE;

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
    /// The lifecycle state; changed only by a transition.
    pub status: ReturnStatus,
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
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
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

impl StateMachine for SupplierReturn {
    type State = ReturnStatus;
    const COLUMN: renox::db::Col<SupplierReturn, ReturnStatus> = SupplierReturn::STATUS;

    fn machine() -> Machine<Self> {
        use ReturnStatus::{Cancelled, Credited, Dispatched, Draft};
        Machine::new()
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
            .terminal()
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
        assert_eq!(RejectedHandling::KeptForCollection.as_str(), "kept_for_collection");
        assert_eq!(WantedOutcome::default().as_str(), "replace");
        assert_eq!(ReturnOrigin::default().as_str(), "rejected_at_receipt");
    }
}
