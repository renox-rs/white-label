//! The kitchen side of an order (06 F&B §7.1 *Kitchen ticket*; base for
//! BL-FNB-04 to BL-FNB-06): the tickets made when an order is accepted, and
//! what starting and bumping them does to the order.
//!
//! - accepting an order makes one ticket per item line;
//! - starting the first ticket runs the order's `start`;
//! - bumping the last open ticket runs `mark_ready` (and `start` first, when
//!   the cook bumped straight from the queue);
//! - a changed order (an amendment, a cancelled line) brings its tickets in
//!   line at once ([`sync_tickets`], BL-FNB-05).

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use wl_kernel::sales::order_desk;

use crate::order::{FnbOrder, FnbOrderStatus};
use crate::order_machine::{MARK_READY, START};
use crate::ticket::{
    BUMP, DEFAULT_STATION, KitchenTicket, START as TICKET_START, TicketStatus, VOID,
};

/// The tickets of the accepted `order`, one per item line; none are made
/// twice.
///
/// # Errors
///
/// Database errors.
pub async fn on_accepted(state: &AppState, order: &FnbOrder) -> Result {
    if !KitchenTicket::of_order(&state.db, order.id)
        .await?
        .is_empty()
    {
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
        TICKET_START => move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START).await,
        BUMP => {
            let tickets = KitchenTicket::of_order(&state.db, ticket.order_id).await?;
            let all_bumped = tickets
                .iter()
                .all(|t| matches!(t.status, TicketStatus::Done | TicketStatus::Voided));
            if !all_bumped {
                // At least the order has started.
                return move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START).await;
            }
            move_order(state, ticket.order_id, FnbOrderStatus::Accepted, START).await?;
            move_order(
                state,
                ticket.order_id,
                FnbOrderStatus::Preparing,
                MARK_READY,
            )
            .await
        }
        _ => Ok(()),
    }
}

/// Brings the tickets of the `accepted` or `preparing` order `order_id` in
/// line with its sales order after an amendment or a cancelled line: a line
/// with nothing open has its queued or cooking ticket voided (a bumped one
/// stays `done`), a changed quantity is changed on its queued ticket, a line
/// added by an amendment gets a queued ticket, and the order whose open
/// tickets are all gone is `ready`.
///
/// # Errors
///
/// Database errors and the tickets' transition failures.
pub(crate) async fn sync_tickets(state: &AppState, order_id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, order_id).await? else {
        return Ok(());
    };
    if !matches!(
        order.status,
        FnbOrderStatus::Accepted | FnbOrderStatus::Preparing
    ) {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    let tickets = KitchenTicket::of_order(&state.db, order.id).await?;
    for line in view.lines.iter().filter(|line| line.is_item) {
        let open = line.open_milli();
        let own: Vec<&KitchenTicket> = tickets
            .iter()
            .filter(|ticket| ticket.sale_line_id == line.id)
            .collect();
        if open == 0 {
            for ticket in own.iter().filter(|ticket| {
                matches!(ticket.status, TicketStatus::Queued | TicketStatus::InPrep)
            }) {
                let _outcome = ticket
                    .transition(VOID)
                    .by_system()
                    .run(state)
                    .await
                    .map_err(TransitionError::into_error)?;
            }
        } else if own.is_empty() {
            let mut ticket = KitchenTicket {
                order_id: order.id,
                sale_line_id: line.id,
                station: DEFAULT_STATION.to_owned(),
                description: line.description.clone(),
                quantity_milli: open,
                status: TicketStatus::Queued,
                ..KitchenTicket::default()
            };
            ticket.insert(&state.db).await?;
        } else {
            for ticket in own.iter().filter(|ticket| {
                ticket.status == TicketStatus::Queued && ticket.quantity_milli != open
            }) {
                KitchenTicket::query()
                    .where_(KitchenTicket::ID.eq(ticket.id))
                    .update(&state.db, &[("quantity_milli", &open)])
                    .await?;
            }
        }
    }
    settle_order(state, order.id).await
}

/// An order that is `preparing` and has nothing left to cook (every ticket
/// is bumped or voided, at least one bumped) is `ready`.
async fn settle_order(state: &AppState, order_id: i64) -> Result {
    let tickets = KitchenTicket::of_order(&state.db, order_id).await?;
    let any_done = tickets
        .iter()
        .any(|ticket| ticket.status == TicketStatus::Done);
    let nothing_left = tickets
        .iter()
        .all(|ticket| matches!(ticket.status, TicketStatus::Done | TicketStatus::Voided));
    if any_done && nothing_left {
        move_order(state, order_id, FnbOrderStatus::Preparing, MARK_READY).await?;
    }
    Ok(())
}
