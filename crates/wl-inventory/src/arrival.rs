//! What happens when stock arrives at a location (a goods receipt is
//! posted, a transfer is received): the quantity that moved into the
//! location of an open stock hold joins that hold as new lines, decided like
//! the others and never available while the hold is open (wl-inventory §4
//! *Stock hold* `add_lines`, issue #230).
//!
//! Decision: the screens warn first ([`crate::hold_service::warnings_for`]);
//! the stock is moved all the same once the user goes on, so this runs for
//! every arrival, never only after a warning. A hold on whole lots needs no
//! lines: it covers the lot wherever it moves.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine};
use crate::hold_service::{add_lines, open_on_location};
use crate::stock_hold::AddedLine;
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

/// `arrivals` arrived at `location_id`, as committed.
///
/// # Errors
///
/// A database error, or a refusal of `add_lines`.
pub async fn arrived(state: &AppState, location_id: i64, arrivals: &[Arrival]) -> Result {
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
    Ok(())
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
    arrived(state, receipt.location_id, &arrivals).await
}

/// The lines of a received transfer arrived at its destination.
///
/// # Errors
///
/// See [`arrived`].
pub async fn on_transfer(state: &AppState, transfer: &Transfer) -> Result {
    let arrivals: Vec<Arrival> = transfer_lines(&state.db, transfer.id)
        .await?
        .iter()
        .map(|line| Arrival {
            item_id: line.item_id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
        })
        .collect();
    arrived(state, transfer.to_location_id, &arrivals).await
}
