//! The stock balance (wl-inventory §3 *Stock balance*): derived from the
//! movements and the holds, never stored. Per item, location and lot (for a
//! lot-tracked item):
//!
//! - *on hand*: what arrived less what left;
//! - *reserved*: set aside for a draft Return to supplier, or for a sales
//!   order line ([`crate::reservation`], BL-SAL-17);
//! - *on hold*: blocked by an open stock hold: the undecided lines of an open
//!   hold on whole lots follow the lot wherever it is, those of an open hold
//!   on a location block that balance only (a line decided `release`,
//!   `discard` or `return_to_supplier` no longer holds anything);
//! - *available*: on hand less reserved less on hold, never below zero.
//!
//! [`candidates`] lists where a picking, an issue or an automatic
//! consumption may take stock from: first expiry first out among the
//! `released` lots of a lot-tracked item, never held quantity.
//!
//! Decision: the functions take a pool or a transaction (`impl Executor`),
//! so a guard reads the same numbers as the effect that follows it.

use std::collections::BTreeSet;

use renox::db::Executor;
use renox::prelude::*;

use crate::item::Item;
use crate::lot::{Lot, LotStatus};
use crate::movement::StockMovement;
use crate::stock_hold::{HoldLineStatus, HoldScope, HoldStatus, StockHold, StockHoldLine};
use crate::supplier_return::{ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine};

/// The balance of an item at a location (and lot), in thousandths of the
/// base unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Balance {
    /// What arrived less what left.
    pub on_hand_milli: i64,
    /// Set aside for a draft Return to supplier.
    pub reserved_milli: i64,
    /// Blocked by an open stock hold.
    pub held_milli: i64,
}

impl Balance {
    /// On hand less reserved less on hold, never below zero.
    pub fn available_milli(&self) -> i64 {
        (self.on_hand_milli - self.reserved_milli - self.held_milli).max(0)
    }
}

/// Where stock can be taken from: a lot (none for an item that is not
/// lot-tracked) and the quantity available there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity available, in thousandths.
    pub available_milli: i64,
}

/// The quantity of `item_id` at `location_id` (of `lot_id` when given), in
/// thousandths: what arrived less what left.
///
/// # Errors
///
/// A database error.
pub async fn on_hand_at<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    let mut conn = db.into_conn();
    let mut arrived = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::TO_LOCATION_ID.eq(location_id));
    let mut left = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(item_id))
        .where_(StockMovement::FROM_LOCATION_ID.eq(location_id));
    if let Some(lot_id) = lot_id {
        arrived = arrived.where_(StockMovement::LOT_ID.eq(lot_id));
        left = left.where_(StockMovement::LOT_ID.eq(lot_id));
    }
    let arrived = arrived
        .sum::<i64, _>(conn.reborrow(), "quantity_milli")
        .await?;
    let left = left
        .sum::<i64, _>(conn.reborrow(), "quantity_milli")
        .await?;
    Ok(arrived - left)
}

/// What is left of lot `lot_id` in all locations, in thousandths.
///
/// # Errors
///
/// A database error.
pub async fn lot_left<'c>(db: impl Executor<'c>, lot_id: i64) -> Result<i64> {
    let mut conn = db.into_conn();
    let arrived = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .where_(StockMovement::TO_LOCATION_ID.is_not_null())
        .sum::<i64, _>(conn.reborrow(), "quantity_milli")
        .await?;
    let left = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .where_(StockMovement::FROM_LOCATION_ID.is_not_null())
        .sum::<i64, _>(conn.reborrow(), "quantity_milli")
        .await?;
    Ok(arrived - left)
}

/// The quantity of lot `lot_id` at each location it is at, in thousandths:
/// `(location, quantity)` for every location with more than nothing.
///
/// # Errors
///
/// A database error.
pub async fn lot_locations<'c>(db: impl Executor<'c>, lot_id: i64) -> Result<Vec<(i64, i64)>> {
    let mut conn = db.into_conn();
    let movements = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .order_by(StockMovement::ID)
        .get(conn.reborrow())
        .await?;
    let mut places: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for movement in &movements {
        if let Some(to) = movement.to_location_id {
            *places.entry(to).or_insert(0) += movement.quantity_milli;
        }
        if let Some(from) = movement.from_location_id {
            *places.entry(from).or_insert(0) -= movement.quantity_milli;
        }
    }
    Ok(places.into_iter().filter(|(_, qty)| *qty > 0).collect())
}

/// What draft returns to supplier and picked dispatches set aside of `item_id` at `location_id`
/// (of `lot_id` when given), in thousandths, and, when no lot is named, what
/// sales orders set aside.
///
/// # Errors
///
/// A database error.
pub async fn reserved_at<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    let mut conn = db.into_conn();
    let drafts = SupplierReturn::query()
        .where_(SupplierReturn::STATUS.eq(ReturnStatus::Draft))
        .where_(SupplierReturn::LOCATION_ID.eq(Some(location_id)))
        .get(conn.reborrow())
        .await?;
    let mut reserved = 0;
    for draft in drafts {
        let lines = SupplierReturnLine::query()
            .where_(SupplierReturnLine::SUPPLIER_RETURN_ID.eq(draft.id))
            .get(conn.reborrow())
            .await?;
        reserved += lines
            .iter()
            .filter(|line| line.origin == ReturnOrigin::FromStock && line.item_id == item_id)
            .filter(|line| lot_id.is_none() || line.lot_id == lot_id)
            .map(|line| line.quantity_milli)
            .sum::<i64>();
    }
    // A picked dispatch keeps its goods reserved until it is dispatched or
    // cancelled (wl-inventory §4 *Dispatch* `pick`).
    let picked = crate::dispatch::Dispatch::query()
        .where_(crate::dispatch::Dispatch::STATUS.eq(crate::dispatch::DispatchStatus::Picked))
        .where_(crate::dispatch::Dispatch::LOCATION_ID.eq(location_id))
        .get(conn.reborrow())
        .await?;
    // The part of a picked line that the order's own reservation of the sale
    // line already covers is counted once (BL-SAL-17).
    let mut covered: std::collections::BTreeMap<(i64, i64), i64> =
        std::collections::BTreeMap::new();
    for dispatch in picked {
        let lines = crate::dispatch::DispatchLine::query()
            .where_(crate::dispatch::DispatchLine::DISPATCH_ID.eq(dispatch.id))
            .where_(crate::dispatch::DispatchLine::ITEM_ID.eq(item_id))
            .get(conn.reborrow())
            .await?;
        for line in lines
            .iter()
            .filter(|line| lot_id.is_none() || line.lot_id == lot_id)
        {
            let mut quantity = line.quantity_milli;
            if lot_id.is_none()
                && dispatch.source_type == wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE
                && let Some(sale_line) = line.sale_line_id
            {
                let key = (dispatch.source_id, sale_line);
                let left = match covered.get(&key) {
                    Some(left) => *left,
                    None => {
                        crate::reservation::reserved_for_line(
                            conn.reborrow(),
                            &dispatch.source(),
                            sale_line,
                            item_id,
                            location_id,
                        )
                        .await?
                    }
                };
                let used = left.min(quantity);
                covered.insert(key, left - used);
                quantity -= used;
            }
            reserved += quantity;
        }
    }
    // What sales orders set aside is by item and location, not by lot
    // (BL-SAL-17).
    if lot_id.is_none() {
        reserved +=
            crate::reservation::sales_reserved(conn.reborrow(), item_id, location_id).await?;
    }
    Ok(reserved)
}

/// The quantity of `item_id` at `location_id` (of `lot_id` when given) that
/// an open stock hold blocks, in thousandths, never more than is on hand.
///
/// A hold on whole lots blocks every lot its lines name, wherever the lot is
/// (the lot may have moved since); a hold on a location blocks the quantity
/// of its lines at that location.
///
/// # Errors
///
/// A database error.
pub async fn held_at<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    let mut conn = db.into_conn();
    let holds = StockHold::query()
        .where_(StockHold::STATUS.eq(HoldStatus::Open))
        .get(conn.reborrow())
        .await?;
    if holds.is_empty() {
        return Ok(0);
    }
    let mut whole_lots: BTreeSet<i64> = BTreeSet::new();
    let mut at_location = 0;
    for hold in &holds {
        let lines = StockHoldLine::query()
            .where_(StockHoldLine::STOCK_HOLD_ID.eq(hold.id))
            .where_(StockHoldLine::ITEM_ID.eq(item_id))
            .where_(StockHoldLine::STATUS.eq(HoldLineStatus::Undecided))
            .get(conn.reborrow())
            .await?;
        match hold.scope_kind {
            HoldScope::Lots => {
                whole_lots.extend(lines.iter().filter_map(|line| line.lot_id));
                // A recall of an item not tracked by lot holds its whole
                // quantity at each location, in lines that name no lot.
                if lot_id.is_none() {
                    at_location += lines
                        .iter()
                        .filter(|line| line.lot_id.is_none() && line.location_id == location_id)
                        .map(|line| line.quantity_milli)
                        .sum::<i64>();
                }
            }
            HoldScope::Location => {
                at_location += lines
                    .iter()
                    .filter(|line| line.location_id == location_id)
                    .filter(|line| lot_id.is_none() || line.lot_id == lot_id)
                    .map(|line| line.quantity_milli)
                    .sum::<i64>();
            }
        }
    }
    let mut held = at_location;
    for lot in whole_lots {
        if lot_id.is_none() || lot_id == Some(lot) {
            held += on_hand_at(conn.reborrow(), item_id, location_id, Some(lot)).await?;
        }
    }
    let on_hand = on_hand_at(conn.reborrow(), item_id, location_id, lot_id).await?;
    Ok(held.min(on_hand).max(0))
}

/// The balance of `item_id` at `location_id` (of `lot_id` when given).
///
/// # Errors
///
/// A database error.
pub async fn balance<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<Balance> {
    let mut conn = db.into_conn();
    Ok(Balance {
        on_hand_milli: on_hand_at(conn.reborrow(), item_id, location_id, lot_id).await?,
        reserved_milli: reserved_at(conn.reborrow(), item_id, location_id, lot_id).await?,
        held_milli: held_at(conn.reborrow(), item_id, location_id, lot_id).await?,
    })
}

/// Where `item` can be taken from at `location_id`, in the order to pick:
/// an item that is not lot-tracked has one candidate; a lot-tracked item has
/// its `released` lots with something available, first expiry first, then
/// the oldest lot (§3 *Picking order*). Held quantity is never offered.
///
/// This is the picking source. The route-level pick (a dispatch refused for
/// held quantity) belongs to BL-STK-21; until it exists the test
/// `the_picking_source_never_offers_held_quantity` proves the source.
///
/// # Errors
///
/// A database error.
pub async fn candidates<'c>(
    db: impl Executor<'c>,
    item: &Item,
    location_id: i64,
) -> Result<Vec<Candidate>> {
    let mut conn = db.into_conn();
    if !item.lot_tracked {
        let found = balance(conn.reborrow(), item.id, location_id, None).await?;
        return Ok(if found.available_milli() > 0 {
            vec![Candidate {
                lot_id: None,
                available_milli: found.available_milli(),
            }]
        } else {
            Vec::new()
        });
    }
    let mut lots = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .where_(Lot::STATUS.eq(LotStatus::Released))
        .get(conn.reborrow())
        .await?;
    // A lot past its expiry date is never offered, whatever its status says.
    let day = wl_kernel::documents::today();
    lots.retain(|lot| lot.expiry_date.is_none_or(|expiry| expiry >= day));
    lots.sort_by_key(|lot| (lot.expiry_date.is_none(), lot.expiry_date, lot.id));
    let mut found = Vec::new();
    for lot in lots {
        let lot_balance = balance(conn.reborrow(), item.id, location_id, Some(lot.id)).await?;
        if lot_balance.available_milli() > 0 {
            found.push(Candidate {
                lot_id: Some(lot.id),
                available_milli: lot_balance.available_milli(),
            });
        }
    }
    Ok(found)
}

/// The quantity of `item` that can be taken at `location_id` now: the sum of
/// [`candidates`].
///
/// # Errors
///
/// A database error.
pub async fn available_to_take<'c>(
    db: impl Executor<'c>,
    item: &Item,
    location_id: i64,
) -> Result<i64> {
    Ok(candidates(db, item, location_id)
        .await?
        .iter()
        .map(|c| c.available_milli)
        .sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_is_on_hand_less_reserved_less_held_and_never_negative() {
        let balance = Balance {
            on_hand_milli: 10_000,
            reserved_milli: 2_000,
            held_milli: 3_000,
        };
        assert_eq!(balance.available_milli(), 5_000);
        let over = Balance {
            on_hand_milli: 1_000,
            reserved_milli: 2_000,
            held_milli: 0,
        };
        assert_eq!(over.available_milli(), 0);
    }
}
