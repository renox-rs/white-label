//! What happens when stock arrives at a location (a goods receipt is
//! posted, a transfer is received): the quantity that moved into the
//! location of an open stock hold joins that hold as new lines, decided like
//! the others and never available while the hold is open (wl-inventory §4
//! *Stock hold* `add_lines`, issue #230). What is left then posts the pending
//! issues waiting at the location, oldest first (§3 *Negative stock*, issue
//! #237).
//!
//! Decision: the screens warn first ([`crate::hold_service::warnings_for`]);
//! the stock is moved all the same once the user goes on, so this runs for
//! every arrival, never only after a warning. A hold on whole lots needs no
//! new lines: it covers the lot wherever it moves. Its lines that were in
//! transit when it was placed follow the stock to the destination of the
//! transfer ([`follow_lots`]), so they can be decided where the stock is.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine};
use crate::hold_service::{add_lines, open_on_location};
use crate::location::Location;
use crate::stock_hold::{
    AddedLine, HoldLineStatus, HoldScope, HoldStatus, StockHold, StockHoldLine, new_line,
};
use crate::transfer::{Transfer, lines_of as transfer_lines};

/// Stock that arrived at a location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

/// `arrivals` arrived at `location_id`, as committed, by `via` (`receipt`
/// or `transfer`).
///
/// # Errors
///
/// A database error, or a refusal of `add_lines`.
pub async fn arrived(
    state: &AppState,
    location_id: i64,
    arrivals: &[Arrival],
    via: &str,
) -> Result {
    let mut by_hold: BTreeMap<i64, Vec<AddedLine>> = BTreeMap::new();
    for arrival in arrivals.iter().filter(|a| a.quantity_milli > 0) {
        for hold in open_on_location(&state.db, location_id, arrival.item_id).await? {
            by_hold.entry(hold.id).or_default().push(AddedLine {
                item_id: arrival.item_id,
                location_id,
                lot_id: arrival.lot_id,
                quantity_milli: arrival.quantity_milli,
            });
        }
    }
    for (hold_id, lines) in by_hold {
        add_lines(state, None, hold_id, lines)
            .await
            .map_err(TransitionError::into_error)?;
    }
    // Held quantity is not available, so only the rest posts what waits.
    let items: Vec<i64> = arrivals
        .iter()
        .filter(|a| a.quantity_milli > 0)
        .map(|a| a.item_id)
        .collect();
    crate::pending_issue::post_on_arrival(state, location_id, &items, via).await
}

/// The accepted lines of a posted goods receipt arrived at its location.
///
/// # Errors
///
/// See [`arrived`].
pub async fn on_receipt(
    state: &AppState,
    receipt: &GoodsReceipt,
    lines: &[GoodsReceiptLine],
) -> Result {
    let arrivals: Vec<Arrival> = lines
        .iter()
        .filter(|line| line.accepted_qty_milli > 0)
        .map(|line| Arrival {
            item_id: line.item_id,
            lot_id: line.lot_id,
            quantity_milli: line.accepted_qty_milli,
        })
        .collect();
    arrived(state, receipt.location_id, &arrivals, "receipt").await
}

/// The undecided lines of open holds on whole lots that were in the transfer's
/// in-transit location follow the lots to the destination: a line is moved
/// (or split, when the transfer carried less) to the destination.
///
/// # Errors
///
/// A database error.
pub async fn follow_lots(state: &AppState, transfer: &Transfer) -> Result {
    let Some(transit) = transfer.transit_location_id else {
        return Ok(());
    };
    let destination = Location::find_or_404(&state.db, transfer.to_location_id).await?;
    let mut tx = state.db.begin().await?;
    for sent in transfer_lines(&mut tx, transfer.id).await? {
        let Some(lot_id) = sent.lot_id else {
            continue;
        };
        let mut left = sent.quantity_milli;
        let held = StockHoldLine::query()
            .where_(StockHoldLine::LOT_ID.eq(Some(lot_id)))
            .where_(StockHoldLine::LOCATION_ID.eq(transit))
            .where_(StockHoldLine::STATUS.eq(HoldLineStatus::Undecided))
            .order_by(StockHoldLine::ID)
            .get(&mut tx)
            .await?;
        for row in held {
            if left <= 0 {
                break;
            }
            let hold = StockHold::find_or_404(&mut tx, row.stock_hold_id).await?;
            if hold.status != HoldStatus::Open || hold.scope_kind != HoldScope::Lots {
                continue;
            }
            let moved = row.quantity_milli.min(left);
            let now = renox::db::now();
            if moved == row.quantity_milli {
                StockHoldLine::query()
                    .where_(StockHoldLine::ID.eq(row.id))
                    .update(
                        &mut tx,
                        &[
                            ("location_id", &destination.id),
                            ("scope_type", &destination.scope_type),
                            ("scope_id", &destination.scope_id),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
            } else {
                StockHoldLine::query()
                    .where_(StockHoldLine::ID.eq(row.id))
                    .update(
                        &mut tx,
                        &[
                            ("quantity_milli", &(row.quantity_milli - moved)),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                new_line(
                    &mut tx,
                    hold.id,
                    row.item_id,
                    destination.id,
                    Some(lot_id),
                    moved,
                )
                .await?;
            }
            left -= moved;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// The lines of a received transfer arrived at its destination.
///
/// # Errors
///
/// See [`arrived`].
pub async fn on_transfer(state: &AppState, transfer: &Transfer) -> Result {
    follow_lots(state, transfer).await?;
    let arrivals: Vec<Arrival> = transfer_lines(&state.db, transfer.id)
        .await?
        .iter()
        .map(|line| Arrival {
            item_id: line.item_id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
        })
        .collect();
    arrived(state, transfer.to_location_id, &arrivals, "transfer").await
}
