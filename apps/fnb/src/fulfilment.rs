//! The order's delivery (06 F&B §7.1 `dispatch` and `complete`; base for
//! BL-FNB-01):
//!
//! - when the order is `ready` and goes out with an own rider, the delivery is
//!   requested from wl-delivery (`DeliveryRequester`): perishable (one attempt,
//!   `WRK.RUL.16`), within the brand's promise (BL-DLV-10), with the cash to
//!   collect on a cash order. The dispatcher puts it on a run;
//! - `DeliveryAssigned` and `RunHandedOver` say which rider drives it;
//! - `DeliveryOutForDelivery` runs the order's `dispatch` as the system, and
//!   `DeliveryDelivered` its `complete`. A manager may also dispatch an order
//!   whose delivery has a rider ([`dispatch`]) and complete one by hand
//!   ([`complete`]);
//! - completing fulfils the sales order (`OrderFulfilment`).
//!
//! A delivery event for an order that is not in the state the transition
//! leaves (a late `DeliveryDelivered` after a failure) is ignored and logged,
//! never applied.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::delivery::{
    DeliveryAssigned, DeliveryDelivered, DeliveryId, DeliveryItem, DeliveryOutForDelivery,
    DeliveryRequest, RunHandedOver, requester,
};
use wl_kernel::sales::{FulfilOrder, FulfilledLine, order_desk, order_fulfilment};

use crate::catalogue::{Brand, DEFAULT_PROMISE_MINUTES};
use crate::notices::tell_customer;
use crate::order::{FnbOrder, FnbOrderStatus};
use crate::order_machine::{
    COMPLETE, COMPLETE_SYSTEM, DISPATCH, DISPATCH_SYSTEM, OrderInput,
};
use crate::texts::english;

/// The subject type of the sale line a delivery line is copied from.
pub const SALE_LINE_SUBJECT_TYPE: &str = "sales.order_line";

/// The delivery promise of the brand of `order`, in minutes.
async fn promise_of(db: &Db, order: &FnbOrder) -> Result<i64> {
    if let Some(id) = order.brand_id
        && let Some(brand) = Brand::find(db, id).await?
        && brand.promise_minutes > 0
    {
        return Ok(brand.promise_minutes);
    }
    Ok(DEFAULT_PROMISE_MINUTES)
}

/// Requests the delivery of the `ready` order `id`: perishable, within the
/// promise of its brand, collecting the total on a cash order. An order that
/// is collected, or has a delivery already, asks for nothing. A refusal of
/// wl-delivery is written as a staff notice and the order stays `ready`.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn request_delivery(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    if !order.is_delivery() || order.delivery_id.is_some() {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    let name = if order.recipient_name.trim().is_empty() {
        english("fnb.delivery.customer", &[])
    } else {
        order.recipient_name.clone()
    };
    let minutes = promise_of(&state.db, &order).await?;
    let placed = order.placed_at.unwrap_or_else(renox::db::now);
    let mut request = DeliveryRequest::new(order.subject(), &name)
        .address(&order.address, &order.area)
        .phone(&order.recipient_phone)
        .perishable()
        .partial_acceptance(false)
        .promise(placed, minutes);
    request.recipient_party = Some(order.party());
    if order.is_cash_on_delivery() {
        // What is still open: a line cancelled or replaced is not paid for.
        request = request.collect(crate::changes::open_total(&view));
    }
    for line in view.lines.iter().filter(|l| l.is_item && l.open_milli() > 0) {
        request = request.item(
            DeliveryItem::new(&line.description, line.open_milli())
                .source_line(Subject::new(SALE_LINE_SUBJECT_TYPE, line.id)),
        );
    }
    match requester(state).request(state, request).await {
        Ok(delivery) => {
            let linked = Some(delivery.0);
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(id))
                .update(&state.db, &[("delivery_id", &linked)])
                .await?;
        }
        Err(error) => {
            eprintln!(
                "fnb: no delivery was requested for order {}: {error}",
                order.number
            );
            crate::notices::tell_managers(
                state,
                &order,
                crate::permissions::ORDERS_MANAGE,
                &english("fnb.delivery.refused_title", &[("number", &order.number)]),
                &english("fnb.delivery.refused_body", &[("number", &order.number)]),
            )
            .await?;
        }
    }
    Ok(())
}

/// The order whose delivery is `delivery`, when the source is an F&B order.
pub(crate) async fn order_of_delivery(
    state: &AppState,
    source: &Subject,
    delivery: DeliveryId,
) -> Result<Option<FnbOrder>> {
    if let Some(order) = FnbOrder::of_source(&state.db, source).await? {
        return Ok(Some(order));
    }
    FnbOrder::query()
        .where_(FnbOrder::DELIVERY_ID.eq(Some(delivery.0)))
        .first(&state.db)
        .await
}

/// `DeliveryAssigned`: the rider and the run of the order's delivery.
pub(crate) async fn on_assigned(state: &AppState, event: DeliveryAssigned) -> Result {
    let Some(order) = FnbOrder::of_source(&state.db, &event.source).await? else {
        return Ok(());
    };
    let (run, rider) = (Some(event.run.0), Some(event.driver));
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(
            &state.db,
            &[
                ("delivery_id", &Some(event.delivery.0)),
                ("run_id", &run),
                ("rider_user_id", &rider),
            ],
        )
        .await?;
    Ok(())
}

/// `RunHandedOver`: the stops that moved to the receiving rider's run.
pub(crate) async fn on_handed_over(state: &AppState, event: RunHandedOver) -> Result {
    for delivery in &event.deliveries {
        let found = FnbOrder::query()
            .where_(FnbOrder::DELIVERY_ID.eq(Some(delivery.0)))
            .first(&state.db)
            .await?;
        if let Some(order) = found {
            let (run, rider) = (Some(event.to_run.0), Some(event.to_driver));
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(order.id))
                .update(&state.db, &[("run_id", &run), ("rider_user_id", &rider)])
                .await?;
        }
    }
    Ok(())
}

/// `DeliveryOutForDelivery`: the run left with the order's delivery.
pub(crate) async fn on_out_for_delivery(
    state: &AppState,
    event: DeliveryOutForDelivery,
) -> Result {
    let Some(order) = order_of_delivery(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    if order.status != FnbOrderStatus::Ready {
        eprintln!(
            "fnb: delivery {} left but order {} is {}: ignored",
            event.delivery, order.number, order.status.as_str()
        );
        return Ok(());
    }
    let _outcome = order
        .transition(DISPATCH_SYSTEM)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// `DeliveryDelivered`: the rider handed the order over.
pub(crate) async fn on_delivered(state: &AppState, event: DeliveryDelivered) -> Result {
    let Some(order) = order_of_delivery(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    if !matches!(
        order.status,
        FnbOrderStatus::Ready | FnbOrderStatus::Dispatched
    ) {
        eprintln!(
            "fnb: delivery {} was delivered but order {} is {}: ignored",
            event.delivery, order.number, order.status.as_str()
        );
        return Ok(());
    }
    let _outcome = order
        .transition(COMPLETE_SYSTEM)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// A manager dispatches the `ready` order `id`: it goes out with a rider.
///
/// # Errors
///
/// `Forbidden` without `orders.manage`; 422 for an order that is not a
/// delivery or has no rider yet (the guard's words); `NotFrom` unless `ready`.
pub async fn dispatch(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(), TransitionError> {
    let order = FnbOrder::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let input = OrderInput {
        rider_assigned: order.rider_user_id.is_some(),
        ..OrderInput::default()
    };
    let _outcome = order
        .transition(DISPATCH)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(())
}

/// A manager completes the `ready` or `dispatched` order `id` by hand.
///
/// # Errors
///
/// `Forbidden` without `orders.manage`; `NotFrom` unless `ready` or
/// `dispatched`.
pub async fn complete(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<(), TransitionError> {
    let order = FnbOrder::find_or_404(&state.db, id)
        .await
        .map_err(TransitionError::Failed)?;
    let _outcome = order.transition(COMPLETE).by(user).run(state).await?;
    Ok(())
}

/// After `complete`: the sales order is fulfilled with the open quantities
/// and the customer's receipt is told.
///
/// # Errors
///
/// Database errors and the sales module's.
pub(crate) async fn after_completed(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    let lines: Vec<FulfilledLine> = view
        .lines
        .iter()
        .filter(|line| line.is_item && line.open_milli() > 0)
        .map(|line| FulfilledLine {
            line: line.id,
            quantity_milli: line.open_milli(),
        })
        .collect();
    if !lines.is_empty() {
        order_fulfilment(state)
            .fulfil(
                state,
                FulfilOrder {
                    order: order.sales_order(),
                    lines,
                    source: order.subject(),
                },
            )
            .await?;
    }
    tell_customer(
        state,
        &order,
        &english("fnb.completed.title", &[("number", &order.number)]),
        &english("fnb.completed.body", &[("number", &order.number)]),
    )
    .await
}
