//! The Stock movement (wl-inventory §3): one immutable quantity change. A
//! movement is never edited or deleted; a mistake is corrected by a reversing
//! movement. The balance of an item at a location is derived from the
//! movements ([`on_hand`]), never stored.
//!
//! Only `receive` is posted so far, by a goods receipt: accepted quantity
//! only. Rejected goods never have a movement.

use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// What a movement does (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MovementType {
    /// Goods come in on a goods receipt.
    #[default]
    Receive,
    /// Stock is issued.
    Issue,
    /// Stock is consumed.
    Consume,
    /// Stock is produced.
    Produce,
    /// Stock leaves a location on a transfer.
    TransferOut,
    /// Stock arrives at a location on a transfer.
    TransferIn,
    /// A count or a correction adjusts stock.
    Adjust,
    /// Stock is lost.
    Wastage,
    /// Goods come back in.
    ReturnIn,
    /// Goods go back to the supplier.
    ReturnOut,
    /// Stock is dispatched to a recipient.
    Dispatch,
}

/// One quantity change. Quantities are thousandths of the item's base unit,
/// money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_stock_movements",
    index(item_id),
    index(to_location_id),
    index(from_location_id),
    index(source_type, source_id)
)]
pub struct StockMovement {
    /// The primary key.
    pub id: i64,
    /// What it does.
    pub movement_type: MovementType,
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// Where the stock left from.
    pub from_location_id: Option<i64>,
    /// Where the stock arrived.
    pub to_location_id: Option<i64>,
    /// The quantity, above 0.
    pub quantity_milli: i64,
    /// The cost of one base unit.
    pub unit_cost: i64,
    /// The value of the movement.
    pub value: i64,
    /// The type of the cost object charged.
    pub cost_object_type: Option<String>,
    /// The id of the cost object charged.
    pub cost_object_id: Option<i64>,
    /// The type of the record that caused it, e.g. `inventory.goods_receipt`.
    pub source_type: String,
    /// The id of the record that caused it.
    pub source_id: i64,
    /// Why, when it needs a reason.
    pub reason: Option<String>,
    /// The user who caused it; `None` for the system.
    pub user_id: Option<i64>,
    /// When it happened.
    pub moved_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockMovement {
    /// A `receive` of `quantity_milli` of `item_id` at `location_id`,
    /// caused by `source`, at `unit_cost` fils a unit worth `value`.
    pub fn receive(
        item_id: i64,
        lot_id: Option<i64>,
        location_id: i64,
        quantity_milli: i64,
        unit_cost: i64,
        value: i64,
        source: &Subject,
        user_id: Option<i64>,
    ) -> Self {
        StockMovement {
            movement_type: MovementType::Receive,
            item_id,
            lot_id,
            to_location_id: Some(location_id),
            quantity_milli,
            unit_cost,
            value,
            source_type: source.subject_type.clone(),
            source_id: source.subject_id,
            user_id,
            moved_at: renox::db::now(),
            ..StockMovement::default()
        }
    }

    /// The record that caused it.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }
}

/// The quantity of `item_id` at `location_id`, in thousandths, read in `db`:
/// what arrived less what left (§3 *Stock balance*, derived, never stored).
pub async fn on_hand(db: &Db, item_id: i64, location_id: i64) -> Result<i64> {
    let arrived = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::TO_LOCATION_ID.eq(location_id))
        .sum::<i64, _>(db, "quantity_milli")
        .await?;
    let left = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::FROM_LOCATION_ID.eq(location_id))
        .sum::<i64, _>(db, "quantity_milli")
        .await?;
    Ok(arrived - left)
}

/// The quantity of `item_id` in all locations, read in `tx`: the base of the
/// moving weighted average (§3 *Costing*).
pub async fn on_hand_everywhere_in(tx: &mut Transaction, item_id: i64) -> Result<i64> {
    let arrived = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::TO_LOCATION_ID.is_not_null())
        .sum::<i64, _>(&mut *tx, "quantity_milli")
        .await?;
    let left = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::FROM_LOCATION_ID.is_not_null())
        .sum::<i64, _>(&mut *tx, "quantity_milli")
        .await?;
    Ok(arrived - left)
}

/// The movements caused by `source`, oldest first.
pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<StockMovement>> {
    StockMovement::query()
        .where_(StockMovement::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(StockMovement::SOURCE_ID.eq(source.subject_id))
        .order_by(StockMovement::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_receive_arrives_at_one_location_from_its_source() {
        let source = Subject::new("inventory.goods_receipt", 5);
        let movement = StockMovement::receive(9, Some(2), 3, 80_000, 1_200, 96_000, &source, Some(7));
        assert_eq!(movement.movement_type, MovementType::Receive);
        assert_eq!(movement.to_location_id, Some(3));
        assert_eq!(movement.from_location_id, None);
        assert_eq!(movement.source(), source);
        assert_eq!((movement.quantity_milli, movement.value), (80_000, 96_000));
    }

    #[test]
    fn the_types_are_stored_in_snake_case() {
        let names: Vec<_> = MovementType::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(names[0], "receive");
        assert!(names.contains(&"transfer_out") && names.contains(&"return_out"));
        assert_eq!(names.len(), 11);
    }
}
