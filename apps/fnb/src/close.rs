//! Closing a failed delivery with the refund decision (06 F&B §7.1
//! `close_failed`, `FNB.ACL.16`, `FNB.ACL.17`; BL-FNB-02).
//!
//! When the food from a failed delivery is back at the outlet
//! (`DeliveryReturned`) or was discarded (`DeliveryDiscarded`), the order is
//! closed as `cancelled`, with a refund decision and a responsible party:
//!
//! - **Decision** ([`proposed_decision`](crate::order_machine::proposed_decision)):
//!   *full refund* of everything paid (items and delivery fee) at once when
//!   the reason is on the outlet's side; *no refund* for a customer-side
//!   reason unless the kitchen manager approves one ([`crate::refunds`]);
//!   *not applicable* on cash on delivery, which collected nothing. The
//!   customer-side default is a setting
//!   ([`FnbSettings::customer_side_refund`](crate::settings::FnbSettings)):
//!   open item L-33 (consumer protection) is with the lawyer.
//! - **Responsible party**: proposed when the delivery failed
//!   ([`crate::failed::change_party`] changes it with a note).
//! - **Wastage, once** (`FNB.ACL.16`): the food is never sold again. With the
//!   policy *return* the sales order is cancelled with outcome *scrap* for its
//!   lines and `wl-inventory` posts the components as wastage with the reason
//!   *failed delivery*. With *discard* the wastage record of the discard is
//!   the posting; the order remembers it (`wastage_ref`, `wastage_posted`),
//!   the stock side of the lines then reports it as consumed and no second
//!   movement is posted.
//! - **Refund**: asked of wl-payments from the order as its source, by the
//!   system (`refunds.approve` of a second user pays it, `FIN.ACL.04`). The
//!   sales order is cancelled with the payment kept (`keep_payment`), so the
//!   refund is the order's decision alone; no `SAL.ACL.05` approval is asked.
//! - A refund the gateway refuses is wl-payments' to retry (BL-PAY-06,
//!   BL-PAY-07); the order stays `cancelled`.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::delivery::{DeliveryDiscarded, DeliveryReturned};
use wl_kernel::entities::LegalEntity;
use wl_kernel::payments::{RefundRequest, RefundSource, payment_recorder};
use wl_kernel::sales::{CancelByVertical, LineOutcomeKind, OrderView, order_desk};

use crate::fulfilment::order_of_delivery;
use crate::notices::{about, tell_managers};
use crate::order::{FnbOrder, FnbOrderStatus};
use crate::order_machine::{CLOSE_FAILED, CLOSE_FAILED_SYSTEM, OrderInput};
use crate::permissions;
use crate::settings::settings;
use crate::texts::english;

/// The cancel reason of an order closed after a failed delivery: the reason of
/// the cancellation event and of the wastage movements.
pub const FAILED_DELIVERY: &str = "failed delivery";

/// What the close is told about the food.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Food {
    /// The wastage record of the delivery's `discard`, if it made one.
    pub wastage_ref: Option<String>,
    /// Whether the discard posted the food as wastage already.
    pub wastage_posted: bool,
}

/// What the sales order says about the money now: what the customer paid.
async fn paid_of(state: &AppState, order: &FnbOrder) -> Result<(i64, Option<OrderView>)> {
    let view = order_desk(state).find(state, order.sales_order()).await?;
    let paid = view
        .as_ref()
        .map_or(order.paid_amount, |view| if view.prepaid { view.gross_total } else { 0 });
    Ok((paid, view))
}

/// Runs `close_failed` on the `delivery_failed` order, as the system or as
/// `user`, with the food as the delivery left it.
async fn close(
    state: &AppState,
    order: &FnbOrder,
    user: Option<&AuthUser>,
    food: Food,
) -> std::result::Result<(), TransitionError> {
    let (paid, _view) = paid_of(state, order)
        .await
        .map_err(TransitionError::Failed)?;
    let input = OrderInput {
        paid_amount: paid,
        customer_side_refund: settings(state).customer_side_refund,
        close_reason: Some(FAILED_DELIVERY.to_owned()),
        wastage_ref: food.wastage_ref,
        wastage_posted: food.wastage_posted,
        ..OrderInput::default()
    };
    let history = english("fnb.close.history", &[("number", &order.number)]);
    let outcome = match user {
        Some(user) => {
            order
                .transition(CLOSE_FAILED)
                .by(user)
                .reason(history)
                .input(&input)
                .run(state)
                .await?
        }
        None => {
            order
                .transition(CLOSE_FAILED_SYSTEM)
                .by_system()
                .reason(history)
                .input(&input)
                .run(state)
                .await?
        }
    };
    let _ = outcome;
    Ok(())
}

/// The `delivery_failed` order whose delivery the event names; an order in
/// any other state is left alone (a second event, a late one).
async fn failed_order(
    state: &AppState,
    source: &wl_kernel::Subject,
    delivery: wl_kernel::delivery::DeliveryId,
) -> Result<Option<FnbOrder>> {
    let Some(order) = order_of_delivery(state, source, delivery).await? else {
        return Ok(None);
    };
    if order.status != FnbOrderStatus::DeliveryFailed {
        eprintln!(
            "fnb: delivery {delivery} was closed but order {} is {}: ignored",
            order.number,
            order.status.as_str()
        );
        return Ok(None);
    }
    Ok(Some(order))
}

/// `DeliveryReturned`: the food is back at the outlet.
pub(crate) async fn on_returned(state: &AppState, event: DeliveryReturned) -> Result {
    let Some(order) = failed_order(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    close(state, &order, None, Food::default())
        .await
        .map_err(TransitionError::into_error)
}

/// `DeliveryDiscarded`: the food was destroyed, with a photo; the wastage
/// record is the posting.
pub(crate) async fn on_discarded(state: &AppState, event: DeliveryDiscarded) -> Result {
    let Some(order) = failed_order(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    let food = Food {
        wastage_ref: event
            .wastage
            .as_ref()
            .map(|subject| format!("{}:{}", subject.subject_type, subject.subject_id)),
        wastage_posted: event.wastage.is_some(),
    };
    close(state, &order, None, food)
        .await
        .map_err(TransitionError::into_error)
}

/// A manager (`orders.manage`) closes the `delivery_failed` order `id` by
/// hand, with the proposed decision.
///
/// # Errors
///
/// `Forbidden` without `orders.manage`; `NotFrom` unless `delivery_failed`.
pub async fn close_by_user(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(), TransitionError> {
    let order = FnbOrder::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    close(state, &order, Some(user), Food::default()).await
}

/// After `close_failed` committed: the sales order is cancelled with the
/// food scrapped (once), and the refund of the decision is asked.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn after_closed(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    if order.status != FnbOrderStatus::Cancelled {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    if view.status != "cancelled" {
        scrap_and_cancel(state, &order, &view).await?;
    }
    if order.refund_amount > 0 && order.refund_id.is_none() {
        refund(state, &order, &view).await?;
    }
    Ok(())
}

/// Cancels the sales order with the payment kept and every line scrapped, and
/// remembers that the food is posted as wastage.
async fn scrap_and_cancel(state: &AppState, order: &FnbOrder, view: &OrderView) -> Result {
    let outcomes = view
        .lines
        .iter()
        .filter(|line| line.is_item && line.open_milli() > 0)
        .map(|line| (line.id, LineOutcomeKind::Scrap))
        .collect();
    let cancelled = order_desk(state)
        .cancel(
            state,
            CancelByVertical {
                order: order.sales_order(),
                reason: FAILED_DELIVERY.to_owned(),
                outcomes,
                keep_payment: true,
                by: None,
            },
        )
        .await;
    match cancelled {
        Ok(()) => {
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(order.id))
                .update(&state.db, &[("wastage_posted", &true)])
                .await?;
        }
        Err(error) => {
            let problem = error.to_string();
            eprintln!(
                "fnb: the sales order of {} could not be cancelled: {problem}",
                order.number
            );
            tell_managers(
                state,
                order,
                permissions::ORDERS_MANAGE,
                &about("fnb.close.cancel_failed_title", order),
                &english(
                    "fnb.close.cancel_failed_body",
                    &[("number", &order.number), ("problem", &problem)],
                ),
            )
            .await?;
        }
    }
    Ok(())
}

/// Asks wl-payments for the refund of the decision, from the order.
async fn refund(state: &AppState, order: &FnbOrder, view: &OrderView) -> Result {
    let reason = english("fnb.close.refund_reason", &[("number", &order.number)]);
    match request_refund(state, order, view, order.refund_amount, &reason, None).await {
        Ok(id) => {
            let linked = Some(id);
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(order.id))
                .update(&state.db, &[("refund_id", &linked)])
                .await?;
        }
        Err(error) => {
            let problem = error.to_string();
            eprintln!("fnb: no refund was asked for order {}: {problem}", order.number);
            tell_managers(
                state,
                order,
                permissions::ORDERS_MANAGE,
                &about("fnb.close.refund_failed_title", order),
                &english(
                    "fnb.close.refund_failed_body",
                    &[("number", &order.number), ("problem", &problem)],
                ),
            )
            .await?;
        }
    }
    Ok(())
}

/// Asks wl-payments for a refund of `amount` fils from the order, by `by`
/// (the system when none): it waits for a second user's approval
/// (`FIN.ACL.04`).
///
/// # Errors
///
/// The refusal of wl-payments (no payments module, an unknown party, an
/// inactive legal entity).
pub(crate) async fn request_refund(
    state: &AppState,
    order: &FnbOrder,
    view: &OrderView,
    amount: i64,
    reason: &str,
    by: Option<i64>,
) -> Result<i64> {
    let entity = match view.legal_entity {
        Some(entity) => entity,
        None => LegalEntity::default_entity(&state.db).await?.id,
    };
    let request = RefundRequest {
        source: RefundSource::Order(view.id.subject()),
        party: order.party(),
        legal_entity_id: entity,
        amount,
        currency: view.currency.clone(),
        method: None,
        gateway_txn: None,
        reason: reason.to_owned(),
    };
    let id = payment_recorder(state)
        .request_refund(state, request, by)
        .await?;
    Ok(id.0)
}
