//! Unlinking a work order from its sales order to stock (07 Manufacturing §4.2
//! row 2, §7.1 *Work order* `unlink`; BL-MFG-02).
//!
//! When the customer cancels but the goods are worth finishing, the work
//! order keeps running and stops belonging to the sales order: its source
//! becomes stock and its output goes to free stock. The planner
//! (`work_orders.release`) runs it with a reason; the system runs it for
//! `MFG.RUL.09` (outcome *finish to stock*, or the line's open quantity cut
//! to 0, BL-MFG-03).
//!
//! - The state, quantity and line do not change; the history row has the
//!   reason (the transition is a self-transition per state, see
//!   [`crate::work_order`]).
//! - A work order already sourced from stock is not offered `unlink`
//!   ([`not_offered`]); a system call on it does nothing.
//! - Output lots reserved for a line that is cancelled (open quantity 0, or
//!   the order cancelled) are released to free stock through the kernel's
//!   `ProductionStock`, in the transaction of the unlink; a reservation for a
//!   line that is still open stays.
//! - Actual cost stays on the work order. A `completed` work order cannot be
//!   unlinked: its output is handled by the sales outcome *return to stock*.
//! - The planners of the plant are told.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::approvals::announce;
use wl_kernel::inventory::LOT_SUBJECT_TYPE;
use wl_kernel::production::{order_lines, production_stock};

use crate::db::{answer, refused};
use crate::events::WorkOrderUnlinked;
use crate::notices;
use crate::permissions;
use crate::texts::english;
use crate::work_order::{OrderInput, WorkOrder, WorkOrderSource, auto, unlink_name};

/// Why `unlink` is not offered for `order`, if it is not.
pub fn not_offered(order: &WorkOrder) -> Option<String> {
    if unlink_name(order.status).is_none() {
        return Some(english("manufacturing.unlink.state", &[]));
    }
    if order.source != WorkOrderSource::SalesOrder {
        return Some(english("manufacturing.unlink.not_linked", &[]));
    }
    None
}

/// Runs the unlink of `order` as `by` (or the system) with `reason`.
async fn unlink_with(
    state: &AppState,
    by: Option<&AuthUser>,
    order: WorkOrder,
    reason: &str,
) -> Result<WorkOrder> {
    let Some(name) = unlink_name(order.status) else {
        return Err(refused(english("manufacturing.unlink.state", &[])));
    };
    let id = order.id;
    // The line is read before the transaction: it is another module's.
    let cancelled_line = match (order.order(), order.sales_order_line_id) {
        (Some(sales), Some(line)) => order_lines(state)
            .line(state, sales, line)
            .await?
            .is_some_and(|info| info.cancelled || info.open_milli <= 0),
        _ => false,
    };
    let sales = order.order();
    let line = order.sales_order_line_id;
    let lot = order.output_lot_id;
    let input = OrderInput {
        by: by.map(|user| user.id),
        ..OrderInput::default()
    };
    let mut tx = state.db.begin().await?;
    let transition = match by {
        Some(user) => order
            .transition(name)
            .by(user)
            .reason(reason.trim())
            .input(&input),
        None => order
            .transition(&auto(name))
            .by_system()
            .reason(reason.trim())
            .input(&input),
    };
    let outcome = transition.run_in(&mut tx).await.map_err(answer)?;
    if cancelled_line && let (Some(sales), Some(line), Some(lot)) = (sales, line, lot) {
        production_stock(state)
            .release_reservation(&mut tx, &Subject::new(LOT_SUBJECT_TYPE, lot), sales, line)
            .await?;
    }
    tx.commit().await?;
    announce(state, &outcome).await?;
    outcome.emit(state).await?;
    let unlinked = WorkOrder::find_or_404(&state.db, id).await?;
    let planners = notices::holders(state, permissions::PLANNER_ROLE, unlinked.plant_id).await?;
    notices::tell(
        state,
        unlinked.subject(),
        &unlinked.number,
        &planners,
        &english("manufacturing.unlink.title", &[("order", &unlinked.number)]),
        &english(
            "manufacturing.unlink.body",
            &[("order", &unlinked.number), ("reason", &reason.trim())],
        ),
    )
    .await?;
    if let (Some(sales), Some(line)) = (sales, line) {
        state
            .emit(WorkOrderUnlinked {
                work_order: unlinked.subject(),
                sales_order: sales.subject(),
                sales_order_line: line,
                reason: reason.trim().to_owned(),
            })
            .await?;
    }
    Ok(unlinked)
}

/// `by` unlinks work order `id` from its sales order with `reason`.
///
/// # Errors
///
/// `Forbidden` without `work_orders.release` for the work order; a 422
/// without a reason, when the source is already stock or unless it is
/// `planned`, `released`, `in_progress` or `on_hold`.
pub async fn unlink(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<WorkOrder> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_RELEASE) {
        return Err(Error::Forbidden);
    }
    if let Some(problem) = not_offered(&order) {
        return Err(refused(problem));
    }
    unlink_with(state, Some(by), order, reason).await
}

/// The system unlinks work order `id` (`MFG.RUL.09`). A work order that is
/// not linked, or has left the states `unlink` keeps, is left alone: the
/// answer is `None` and nothing changes (replays are harmless).
///
/// # Errors
///
/// 422 without a reason.
pub async fn unlink_by_system(
    state: &AppState,
    id: i64,
    reason: &str,
) -> Result<Option<WorkOrder>> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if not_offered(&order).is_some() {
        return Ok(None);
    }
    unlink_with(state, None, order, reason).await.map(Some)
}
