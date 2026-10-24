//! The output lot of a work order that serves a sale line is set aside for
//! that line (07 Manufacturing §7.1 *Work order* `unlink`, `MFG.RUL.09`;
//! BL-MFG-02 point 5, BL-MFG-03), through the kernel's `ProductionStock`.
//!
//! - Every good quantity reported (and every void of one) brings the
//!   reservation of the lot to `min(good output - quantity for stock, open
//!   quantity of the line)`, in the transaction of the report. The output a
//!   decrease of the order turned into *for stock* (`MFG.RUL.09`) is free
//!   stock from the moment it is reported; so is anything above the line's
//!   open quantity.
//! - Completing the work order checks it once more; the lot (in `quarantine`
//!   until QC releases it) stays reserved for the line.
//! - `unlink` and the abort give the lot back
//!   ([`crate::unlink`], [`crate::abort`]); a work order that is not linked
//!   (source stock) reserves nothing.
//! - A line served by several work orders is capped per work order at the
//!   line's open quantity, not across them.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::production::{ReserveOutput, order_lines, production_stock};

use crate::report::ProductionReport;
use crate::work_order::WorkOrder;

/// The open quantity of the line `order` serves, in thousandths, read before
/// a transaction opens (the line is another module's). `None` when the order
/// is not linked or the sales side does not know the line.
///
/// # Errors
///
/// The sales side's.
pub async fn open_of(state: &AppState, order: &WorkOrder) -> Result<Option<i64>> {
    let (Some(sales), Some(line)) = (order.order(), order.sales_order_line_id) else {
        return Ok(None);
    };
    Ok(order_lines(state)
        .line(state, sales, line)
        .await?
        .map(|info| if info.cancelled { 0 } else { info.open_milli }))
}

/// How much of the output of `order` (with `good_milli` made) the line holds:
/// the good output less the part for stock, up to the line's open quantity.
pub fn target(order: &WorkOrder, good_milli: i64, open: Option<i64>) -> i64 {
    let held = (good_milli - order.quantity_for_stock_milli).max(0);
    open.map_or(held, |open| held.min(open.max(0)))
}

/// Brings the reservation of the output lot of `order` to its target, as the
/// stock sees it. Returns whether a reservation is active afterwards.
///
/// `good_milli` is the good output the work order holds now, `lot` its output
/// lot and `location` where the finished goods are; `open` is [`open_of`].
///
/// # Errors
///
/// The stock's.
pub async fn sync(
    state: &AppState,
    tx: &mut Transaction,
    order: &WorkOrder,
    good_milli: i64,
    lot: Option<&Subject>,
    location: Option<&Subject>,
    open: Option<i64>,
) -> Result<bool> {
    let (Some(sales), Some(line), Some(lot), Some(location)) =
        (order.order(), order.sales_order_line_id, lot, location)
    else {
        return Ok(false);
    };
    production_stock(state)
        .reserve_output(
            tx,
            ReserveOutput {
                source: order.subject(),
                item: Subject::new(ITEM_SUBJECT_TYPE, order.item_id),
                lot: lot.clone(),
                location: location.clone(),
                order: sales,
                line,
                quantity_milli: target(order, good_milli, open),
            },
        )
        .await
}

/// Where the finished goods of `order` were last put, `inventory.location:N`.
///
/// # Errors
///
/// A database error.
pub async fn place_of<'c>(
    db: impl renox::db::Executor<'c>,
    order: &WorkOrder,
) -> Result<Option<Subject>> {
    Ok(ProductionReport::query()
        .where_(ProductionReport::WORK_ORDER_ID.eq(order.id))
        .where_(ProductionReport::LOCATION_ID.is_not_null())
        .order_by_desc(ProductionReport::ID)
        .first(db)
        .await?
        .and_then(|report| report.location_id)
        .map(|id| Subject::new(LOCATION_SUBJECT_TYPE, id)))
}

/// Checks the reservation of `order` on its own (outside any report): the
/// completion, and a change of the part for stock. Does nothing for a work
/// order that is not linked or has no output lot yet.
///
/// # Errors
///
/// The stock's.
pub async fn resync(state: &AppState, order: &WorkOrder) -> Result<bool> {
    let Some(lot) = order
        .output_lot_id
        .map(|lot| Subject::new(LOT_SUBJECT_TYPE, lot))
    else {
        return Ok(false);
    };
    if order.order().is_none() {
        return Ok(false);
    }
    let open = open_of(state, order).await?;
    let location = place_of(&state.db, order).await?;
    let mut tx = state.db.begin().await?;
    let active = sync(
        state,
        &mut tx,
        order,
        order.good_milli,
        Some(&lot),
        location.as_ref(),
        open,
    )
    .await?;
    tx.commit().await?;
    Ok(active)
}
