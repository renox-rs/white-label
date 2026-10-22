//! What happens after a transition or an event of another module commits
//! (06 F&B §7.1): work that reaches other modules runs here, in listeners that
//! re-read the row, never inside an effect.
//!
//! - `SalesOrderPlaced`, `SalesOrderConfirmed` and `SalesOrderRejected` start
//!   and follow the order ([`crate::placing`]);
//! - after `accept` the tickets are made, after `mark_ready` the delivery is
//!   requested, after `fail_delivery` the customer is told and `FNB.RUL.14`
//!   counts, after `close_failed` the sales order is cancelled and the refund
//!   asked, after `complete` the sales order is fulfilled;
//! - the delivery's events follow the order: `DeliveryAssigned`,
//!   `RunHandedOver`, `DeliveryOutForDelivery`, `DeliveryDelivered`,
//!   `DeliveryAttemptFailed`, `DeliveryReturned` and `DeliveryDiscarded`;
//! - `RefundApproved` records a manager-approved refund on the order;
//! - starting and bumping a ticket moves the order ([`crate::kitchen`]).

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::delivery::{
    DeliveryAssigned, DeliveryAttemptFailed, DeliveryDelivered, DeliveryDiscarded,
    DeliveryOutForDelivery, DeliveryReturned, RunHandedOver,
};
use wl_kernel::payments::RefundApproved;
use wl_kernel::sales::{SalesOrderConfirmed, SalesOrderPlaced, SalesOrderRejected};

use crate::order::FnbOrder;
use crate::order_machine::{
    ACCEPT, CLOSE_FAILED, CLOSE_FAILED_SYSTEM, COMPLETE, COMPLETE_SYSTEM, FAIL_DELIVERY,
    MARK_READY,
};
use crate::ticket::KitchenTicket;
use crate::{close, failed, fulfilment, kitchen, placing, refunds};

/// What follows a committed transition of an order.
async fn after_order(state: &AppState, event: &Transitioned<FnbOrder>) -> Result {
    match event.transition.as_str() {
        ACCEPT => match FnbOrder::find(&state.db, event.key).await? {
            Some(order) => kitchen::on_accepted(state, &order).await,
            None => Ok(()),
        },
        MARK_READY => fulfilment::request_delivery(state, event.key).await,
        FAIL_DELIVERY => failed::after_failed(state, event.key).await,
        CLOSE_FAILED | CLOSE_FAILED_SYSTEM => close::after_closed(state, event.key).await,
        COMPLETE | COMPLETE_SYSTEM => fulfilment::after_completed(state, event.key).await,
        _ => Ok(()),
    }
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SalesOrderPlaced, state: AppState| async move {
        placing::on_placed(&state, event).await
    });
    app.listen(|event: SalesOrderConfirmed, state: AppState| async move {
        placing::on_confirmed(&state, event).await
    });
    app.listen(|event: SalesOrderRejected, state: AppState| async move {
        placing::on_rejected(&state, event).await
    });
    app.listen(
        |event: Transitioned<FnbOrder>, state: AppState| async move {
            after_order(&state, &event).await
        },
    );
    app.listen(
        |event: Transitioned<KitchenTicket>, state: AppState| async move {
            kitchen::on_ticket(&state, &event).await
        },
    );
    app.listen(|event: DeliveryAssigned, state: AppState| async move {
        fulfilment::on_assigned(&state, event).await
    });
    app.listen(|event: RunHandedOver, state: AppState| async move {
        fulfilment::on_handed_over(&state, event).await
    });
    app.listen(|event: DeliveryOutForDelivery, state: AppState| async move {
        fulfilment::on_out_for_delivery(&state, event).await
    });
    app.listen(|event: DeliveryDelivered, state: AppState| async move {
        fulfilment::on_delivered(&state, event).await
    });
    app.listen(|event: DeliveryAttemptFailed, state: AppState| async move {
        failed::on_attempt_failed(&state, event).await
    });
    app.listen(|event: DeliveryReturned, state: AppState| async move {
        close::on_returned(&state, event).await
    });
    app.listen(|event: DeliveryDiscarded, state: AppState| async move {
        close::on_discarded(&state, event).await
    });
    app.listen(|event: RefundApproved, state: AppState| async move {
        refunds::on_refund_approved(&state, event).await
    });
}
