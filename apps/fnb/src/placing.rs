//! How an F&B order starts (06 F&B §7.1 *Order*; base for BL-FNB-01 to
//! BL-FNB-03).
//!
//! The order data is the sales order's. The fulfilment record is made when
//! `wl-sales` says the order was placed or confirmed, through the kernel's
//! events, and follows the sales order afterwards:
//!
//! - `SalesOrderPlaced` (a direct order that waits for the outlet to accept
//!   it) makes the order `placed`;
//! - `SalesOrderConfirmed` makes it `accepted`: the order was accepted, or it
//!   came from a channel that needs no acceptance (an aggregator's tablet, the
//!   phone) and starts here. Its kitchen tickets are made;
//! - `SalesOrderRejected` rejects it.
//!
//! The Accept button asks the sales order desk (`sales_orders.accept`), so the
//! two orders never disagree about who accepted. The record needs a delivery
//! address for an own-rider delivery, which the sales order does not hold:
//! [`set_details`] takes it (the order page's form, the cashier on the phone).

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyChannel;
use wl_kernel::sales::{
    OrderView, SalesOrderConfirmed, SalesOrderId, SalesOrderPlaced, SalesOrderRejected,
    order_desk,
};

use crate::catalogue::{MenuItem, Outlet};
use crate::order::{FnbOrder, FnbOrderStatus, kind};
use crate::order_machine::{ACCEPT, OrderInput, REJECT};
use crate::texts::english;

/// The channels whose orders an own rider takes to the customer's door. The
/// others (an aggregator's tablet, the counter) are collected.
pub fn kind_for(channel: &str) -> &'static str {
    match channel {
        "website" | "phone" | "portal" => kind::DELIVERY,
        _ => kind::PICKUP,
    }
}

/// The name and the phone of the customer `party`, as the party directory
/// knows them; empty when it knows none.
async fn recipient_of(state: &AppState, party: PartyId) -> Result<(String, String)> {
    let directory = wl_kernel::parties::directory(state);
    let name = directory
        .find(&state.db, party)
        .await?
        .map(|summary| summary.display_name)
        .unwrap_or_default();
    let phone = match directory.contacts(&state.db, party).await? {
        Some(contacts) => contacts
            .point_for(NotifyChannel::Sms)
            .map(|point| point.address.clone())
            .unwrap_or_default(),
        None => String::new(),
    };
    Ok((name, phone))
}

/// The brand the order was placed with: the brand of the first menu item of
/// its lines.
async fn brand_of(db: &Db, view: &OrderView) -> Result<Option<i64>> {
    for line in view.lines.iter().filter(|line| line.is_item) {
        if let Some(item) = MenuItem::by_code(db, &line.item_code).await?
            && item.brand_id.is_some()
        {
            return Ok(item.brand_id);
        }
    }
    Ok(None)
}

/// Makes the F&B order of the sales order `id`, in `status` (`placed` or
/// `accepted`); the order that exists already is returned as it is. The
/// outlet is the first active one: the deployment with several outlets
/// assigns it when the channel says which kitchen cooks.
///
/// # Errors
///
/// Database errors.
pub async fn open(
    state: &AppState,
    id: SalesOrderId,
    status: FnbOrderStatus,
) -> Result<Option<FnbOrder>> {
    if let Some(existing) = FnbOrder::of_sales_order(&state.db, id.0).await? {
        return Ok(Some(existing));
    }
    let Some(view) = order_desk(state).find(state, id).await? else {
        return Ok(None);
    };
    let outlet = Outlet::query()
        .where_(Outlet::ACTIVE.eq(true))
        .order_by(Outlet::ID)
        .first(&state.db)
        .await?;
    let brand_id = brand_of(&state.db, &view).await?;
    let (name, phone) = recipient_of(state, view.party).await?;
    let now = renox::db::now();
    let mut order = FnbOrder {
        number: view.number.clone(),
        sales_order_id: id.0,
        party_id: view.party.0,
        brand_id,
        outlet_id: outlet.map(|outlet| outlet.id),
        channel: view.channel.clone(),
        kind: kind_for(&view.channel).to_owned(),
        payment_mode: view.payment_mode.clone(),
        paid_amount: if view.prepaid { view.gross_total } else { 0 },
        total: view.gross_total,
        recipient_name: name,
        recipient_phone: phone,
        placed_at: Some(now),
        accepted_at: (status == FnbOrderStatus::Accepted).then_some(now),
        status,
        ..FnbOrder::default()
    };
    // The history starts with the order's first state.
    let mut tx = state.db.begin().await?;
    order
        .insert_in_state(&mut tx, status, &english("fnb.placing.created", &[]))
        .await?;
    tx.commit().await?;
    Ok(Some(order))
}

/// `SalesOrderPlaced`: the order waits for the outlet to accept it.
pub(crate) async fn on_placed(state: &AppState, event: SalesOrderPlaced) -> Result {
    open(state, event.order, FnbOrderStatus::Placed).await?;
    Ok(())
}

/// `SalesOrderConfirmed`: accepted, or placed on a channel that needs no
/// acceptance.
pub(crate) async fn on_confirmed(state: &AppState, event: SalesOrderConfirmed) -> Result {
    match FnbOrder::of_sales_order(&state.db, event.order.0).await? {
        Some(order) if order.status == FnbOrderStatus::Placed => {
            let _outcome = order
                .transition(ACCEPT)
                .by_system()
                .run(state)
                .await
                .map_err(TransitionError::into_error)?;
            Ok(())
        }
        // The order moved on already: a repeated event changes nothing.
        Some(_) => Ok(()),
        None => {
            if let Some(order) = open(state, event.order, FnbOrderStatus::Accepted).await? {
                crate::kitchen::on_accepted(state, &order).await?;
            }
            Ok(())
        }
    }
}

/// `SalesOrderRejected`: the outlet did not accept the order.
pub(crate) async fn on_rejected(state: &AppState, event: SalesOrderRejected) -> Result {
    let Some(order) = FnbOrder::of_sales_order(&state.db, event.order.0).await? else {
        return Ok(());
    };
    if order.status != FnbOrderStatus::Placed {
        return Ok(());
    }
    let _outcome = order
        .transition(REJECT)
        .by_system()
        .reason(event.reason.clone())
        .input(&OrderInput {
            close_reason: Some(event.reason),
            ..OrderInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// What the order page's form sets: how the order reaches the customer and
/// where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    /// `delivery` or `pickup`.
    pub kind: String,
    /// Who receives it.
    pub recipient_name: String,
    /// Their phone.
    pub recipient_phone: String,
    /// The address, for a delivery.
    pub address: String,
    /// The area.
    pub area: String,
}

/// Sets the delivery details of the order `id` while it is not out of the
/// door; the delivery already requested for it takes the new address.
///
/// # Errors
///
/// 422 for an order that is finished or out for delivery, an unknown kind, or
/// a delivery with no recipient or no address.
pub async fn set_details(state: &AppState, id: i64, details: &Details) -> Result<FnbOrder> {
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    if !matches!(
        order.status,
        FnbOrderStatus::Placed
            | FnbOrderStatus::Accepted
            | FnbOrderStatus::Preparing
            | FnbOrderStatus::Ready
    ) {
        return Err(refused(english("fnb.details.closed", &[])));
    }
    let chosen = details.kind.trim().to_owned();
    if chosen != kind::DELIVERY && chosen != kind::PICKUP {
        return Err(refused(english("fnb.details.kind", &[])));
    }
    if chosen == kind::DELIVERY
        && (details.address.trim().is_empty() || details.recipient_name.trim().is_empty())
    {
        return Err(refused(english("fnb.details.address_needed", &[])));
    }
    let name = details.recipient_name.trim().to_owned();
    let phone = details.recipient_phone.trim().to_owned();
    let address = details.address.trim().to_owned();
    let area = details.area.trim().to_owned();
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("kind", &chosen),
                ("recipient_name", &name),
                ("recipient_phone", &phone),
                ("address", &address),
                ("area", &area),
            ],
        )
        .await?;
    if let Some(delivery) = order.delivery_id {
        wl_kernel::delivery::requester(state)
            .update_address(
                state,
                wl_kernel::delivery::DeliveryId(delivery),
                &address,
                &area,
            )
            .await?;
    }
    FnbOrder::find_or_404(&state.db, id).await
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}
