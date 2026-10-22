//! The kitchen side of an order (06 F&B §7.1 *Kitchen ticket*; base for
//! BL-FNB-04 to BL-FNB-06): the tickets made when an order is accepted, and
//! what starting and bumping them does to the order.
//!
//! - accepting an order makes one ticket per item line;
//! - starting the first ticket runs the order's `start`;
//! - bumping the last open ticket runs `mark_ready` (and `start` first, when
//!   the cook bumped straight from the queue).

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::sales::order_desk;

use crate::order::{FnbOrder, FnbOrderStatus};
use crate::order_machine::{MARK_READY, START};
use crate::ticket::{BUMP, DEFAULT_STATION, KitchenTicket, START as TICKET_START, TicketStatus};

/// The tickets of the accepted `order`, one per item line; none are made
/// twice.
///
/// # Errors
///
/// Database errors.
pub async fn on_accepted(state: &AppState, order: &FnbOrder) -> Result {
    if !KitchenTicket::of_order(&state.db, order.id).await?.is_empty() {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    for line in view
        .lines
        .iter()
        .filter(|line| line.is_item && line.open_milli() > 0)
    {
        let mut ticket = KitchenTicket {
            order_id: order.id,
            sale_line_id: line.id,
            station: DEFAULT_STATION.to_owned(),
            description: line.description.clone(),
            quantity_milli: line.open_milli(),
            status: TicketStatus::Queued,
            ..KitchenTicket::default()
        };
        ticket.insert(&state.db).await?;
    }
    Ok(())
}

/// Runs the system transition `name` of the order `id` when it is in `from`.
async fn move_order(state: &AppState, id: i64, from: FnbOrderStatus, name: &str) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    if order.status != from {
        return Ok(());
    }
    let _outcome = order
        .transition(name)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// After a ticket transition committed: the first start starts the order, the
/// last bump makes it ready.
///
/// # Errors
///
/// Database errors and the order's transitions' failures.
pub(crate) async fn on_ticket(state: &AppState, event: &Transitioned<KitchenTicket>) -> Result {
    let Some(ticket) = KitchenTicket::find(&state.db, event.key).await? else {
        return Ok(());
    };
    // A change that waits for a manager was asked before this line started.
    if matches!(event.transition.as_str(), TICKET_START | BUMP) {
        crate::cancel::refuse_waiting(state, ticket.order_id).await?;
    }
    match event.transition.as_str() {
        TICKET_START => {
            move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START).await
        }
        BUMP => {
            let tickets = KitchenTicket::of_order(&state.db, ticket.order_id).await?;
            let all_bumped = tickets
                .iter()
                .all(|t| matches!(t.status, TicketStatus::Done | TicketStatus::Voided));
            if !all_bumped {
                // At least the order has started.
                return move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START)
                    .await;
            }
            move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START).await?;
            move_order(state, ticket.order_id, FnbOrderStatus::Preparing, MARK_READY).await
        }
        _ => Ok(()),
    }
}
