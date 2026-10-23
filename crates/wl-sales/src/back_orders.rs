//! Back-orders (wl-sales §4.2 *Back-orders*, §7 *Back-orders allowed per
//! channel*; BL-SAL-17): a line without stock is back-ordered instead of
//! refused, and arriving stock is reserved for the oldest back-orders first.
//!
//! - When an order is confirmed on a channel with the setting on
//!   ([`SalesConfig::back_orders_allowed`](crate::config::SalesConfig)), each
//!   item line asks the kernel's
//!   [`StockReservations`](wl_kernel::backorders::StockReservations) for its
//!   open quantity: what is available is reserved (`reserved_milli`), the rest
//!   is back-ordered (`backordered_milli`). The order stays `confirmed` and
//!   its other lines are fulfilled as usual. With the setting off nothing is
//!   reserved or back-ordered.
//! - [`StockArrived`] (a goods receipt or a transfer into a location) serves
//!   the open back-orders of that location, oldest first: by the second the
//!   order was confirmed, then order number, then line number. An order on
//!   `credit_hold` is skipped. A pass runs once per arrival and line (the
//!   rows of [`BackOrderServe`]). The customer gets one message per order
//!   when any of its lines was reserved.
//! - [`order_released`] is the seam for `release` (BL-SAL-11, not built on
//!   this branch): it reserves the order's open back-orders at once from the
//!   stock then available, in line order.
//! - [`StockArrivalReversed`] gives back what was reserved from the receipt
//!   and back-orders those quantities again.
//! - A cancelled line takes its quantity off the back-order first, then off
//!   the reservation; a cancelled or rejected order gives everything back;
//!   a hand-over takes its quantity off the reservation first.
//! - [`BackOrderDemand`] tells wl-inventory the open back-ordered quantity
//!   per item and location for `STK.RUL.04`.
//!
//! Decisions: the order's location is the one the first reservation used (the
//! inventory module's selling location unless the order names one); an order
//! with no location (stock is not managed) is never served.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transitioned;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::backorders::{
    BackOrderDemand, BackOrderFuture, BackOrdered, ReserveStock, StockArrivalReversed,
    StockArrived, provide_back_orders, stock_reservations,
};
use wl_kernel::inventory::LOCATION_SUBJECT_TYPE;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::sales::{FulfilOrder, SALES_ORDER_SUBJECT_TYPE};

use crate::config::config_of;
use crate::sale_line::{KIND_ITEM, SaleLine};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::{
    ACCEPT, CANCEL, CANCEL_LINE, CANCEL_LINE_PARTLY, CONFIRM_PAYMENT, CancelLineInput, FULFIL,
    FULFIL_LAST, REJECT, SUBMIT,
};
use crate::texts::english;

/// The customer is told that back-ordered goods were reserved for the order.
pub const BACK_ORDER_RESERVED: &str = "sales.back_order_reserved";

/// What one arrival of stock reserved for one line (one row per arrival and
/// line).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_back_order_serves",
    unique(source_type, source_id, line_id),
    index(order_id)
)]
pub struct BackOrderServe {
    /// The primary key.
    pub id: i64,
    /// The type of what brought the stock.
    pub source_type: String,
    /// The id of what brought the stock.
    pub source_id: i64,
    /// The item.
    pub item_code: String,
    /// The order.
    pub order_id: i64,
    /// The order line.
    pub line_id: i64,
    /// What was reserved, in thousandths.
    pub quantity_milli: i64,
    /// When the receipt was reversed and the reservation given back.
    pub reversed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The notification type of the message to the customer.
pub fn notice_type() -> NotificationTypeDef {
    let subject = english("wl_sales.notices.back_order_reserved_subject", &[]);
    let body = english("wl_sales.notices.back_order_reserved_body", &[]);
    NotificationTypeDef::new(BACK_ORDER_RESERVED, "sales")
        .group("sales")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// `1.5` for 1500 thousandths, `2` for 2000.
fn quantity_text(milli: i64) -> String {
    if milli % 1000 == 0 {
        (milli / 1000).to_string()
    } else {
        format!("{}.{:03}", milli / 1000, milli % 1000)
            .trim_end_matches('0')
            .to_owned()
    }
}

/// The item lines of `order`, in order.
async fn item_lines(state: &AppState, order: &SalesOrder) -> Result<Vec<SaleLine>> {
    let mut lines = SaleLine::of(&state.db, &order.subject()).await?;
    lines.retain(|line| line.kind == KIND_ITEM);
    Ok(lines)
}

/// Moves `reserved` and `backordered` thousandths of line `line` to the
/// reservation side (positive `reserve`) in one statement; false when the
/// line no longer has that much back-ordered.
async fn move_to_reserved(state: &AppState, line: i64, quantity: i64) -> Result<bool> {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET reserved_milli = reserved_milli + ?, \
         backordered_milli = backordered_milli - ?, updated_at = ? \
         WHERE id = ? AND backordered_milli >= ?",
        SaleLine::TABLE
    ))
    .bind(quantity)
    .bind(quantity)
    .bind(renox::db::now())
    .bind(line)
    .bind(quantity)
    .execute(&state.db)
    .await?;
    Ok(changed > 0)
}

/// The request to reserve `quantity` of `line` for `order` at its location.
fn request_for(order: &SalesOrder, line: &SaleLine, quantity: i64) -> ReserveStock {
    let request = ReserveStock::new(&line.item_code, quantity, order.subject(), line.id);
    match order.location_id {
        Some(id) => request.at(Subject::new(LOCATION_SUBJECT_TYPE, id)),
        None => request,
    }
}

/// Reserves the back-ordered quantity of `line` as far as stock is
/// available; records the arrival `source` that served it. Returns what was
/// reserved.
async fn take(
    state: &AppState,
    order: &SalesOrder,
    line: &SaleLine,
    source: Option<&Subject>,
) -> Result<i64> {
    if line.backordered_milli <= 0 {
        return Ok(0);
    }
    let request = request_for(order, line, line.backordered_milli);
    let got = stock_reservations(state)
        .reserve(state, request)
        .await?
        .reserved_milli;
    if got <= 0 {
        return Ok(0);
    }
    if !move_to_reserved(state, line.id, got).await? {
        stock_reservations(state)
            .release(state, &order.subject(), Some(line.id), Some(got))
            .await?;
        return Ok(0);
    }
    if let Some(source) = source {
        BackOrderServe::create(
            &state.db,
            BackOrderServe {
                source_type: source.subject_type.clone(),
                source_id: source.subject_id,
                item_code: line.item_code.clone(),
                order_id: order.id,
                line_id: line.id,
                quantity_milli: got,
                ..BackOrderServe::default()
            },
        )
        .await?;
    }
    Ok(got)
}

/// The order was confirmed: on a channel with back-orders allowed each item
/// line reserves what is available and back-orders the rest. Nothing on other
/// channels.
///
/// # Errors
///
/// A database error.
pub async fn on_confirmed(state: &AppState, order_id: i64) -> Result {
    let Some(mut order) = SalesOrder::find(&state.db, order_id).await? else {
        return Ok(());
    };
    if !config_of(state).back_orders_allowed(&order.channel) {
        return Ok(());
    }
    for line in item_lines(state, &order).await? {
        let wanted = line.open_milli() - line.reserved_milli - line.backordered_milli;
        if wanted <= 0 {
            continue;
        }
        let got = stock_reservations(state)
            .reserve(state, request_for(&order, &line, wanted))
            .await?;
        if order.location_id.is_none() {
            if let Some(location) = got.location {
                let id = Some(location.subject_id);
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(order.id))
                    .update(&state.db, &[("location_id", &id)])
                    .await?;
                order.location_id = id;
            }
        }
        let reserved = line.reserved_milli + got.reserved_milli;
        let backordered = line.backordered_milli + (wanted - got.reserved_milli);
        SaleLine::query()
            .where_(SaleLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("reserved_milli", &reserved),
                    ("backordered_milli", &backordered),
                ],
            )
            .await?;
    }
    Ok(())
}

/// Tells the customer of `order` that goods were reserved for `served`
/// (item code and quantity); one message per order.
async fn tell(state: &AppState, order: &SalesOrder, served: &[(String, i64)]) -> Result {
    let lines = served
        .iter()
        .map(|(code, milli)| format!("{code} x {}", quantity_text(*milli)))
        .collect::<Vec<_>>()
        .join(", ");
    let request = NotifyRequest::new(BACK_ORDER_RESERVED, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({ "number": order.number, "lines": lines }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Stock arrived at `location`: reserves it for the open back-orders of
/// `item_code` there, oldest first, once per arrival `source`. Returns the
/// number of lines served.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn serve(
    state: &AppState,
    location: &Subject,
    item_code: &str,
    source: &Subject,
) -> Result<usize> {
    let done = BackOrderServe::query()
        .where_(BackOrderServe::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(BackOrderServe::SOURCE_ID.eq(source.subject_id))
        .where_(BackOrderServe::ITEM_CODE.eq(item_code.to_owned()))
        .first(&state.db)
        .await?;
    if done.is_some() {
        return Ok(0);
    }
    let lines = SaleLine::query()
        .where_(SaleLine::SUBJECT_TYPE.eq(SALES_ORDER_SUBJECT_TYPE))
        .where_(SaleLine::ITEM_CODE.eq(item_code.to_owned()))
        .where_(SaleLine::BACKORDERED_MILLI.gt(0i64))
        .get(&state.db)
        .await?;
    let mut ranked: Vec<(i64, String, i64, SalesOrder, SaleLine)> = Vec::new();
    for line in lines {
        let Some(order) = SalesOrder::find(&state.db, line.subject_id).await? else {
            continue;
        };
        let serves = matches!(
            order.status,
            SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled
        );
        if !serves || order.location_id != Some(location.subject_id) {
            continue;
        }
        let at = order.confirmed_at.map_or(0, |at| at.timestamp());
        ranked.push((at, order.number.clone(), line.line_no, order, line));
    }
    ranked.sort_by(|a, b| (a.0, &a.1, a.2).cmp(&(b.0, &b.1, b.2)));
    let mut served: BTreeMap<i64, (SalesOrder, Vec<(String, i64)>)> = BTreeMap::new();
    let mut count = 0;
    for (_, _, _, order, line) in ranked {
        let got = take(state, &order, &line, Some(source)).await?;
        if got > 0 {
            count += 1;
            served
                .entry(order.id)
                .or_insert_with(|| (order.clone(), Vec::new()))
                .1
                .push((line.item_code.clone(), got));
        }
        if got < line.backordered_milli {
            // What was available is used up: nothing newer may be served.
            break;
        }
    }
    for (order, lines) in served.values() {
        tell(state, order, lines).await?;
    }
    Ok(count)
}

/// The seam for `release` (BL-SAL-11): the order `order_id` left
/// `credit_hold`; its open back-orders are reserved at once from the stock
/// then available at its location, in line order. Returns the quantity
/// reserved. The customer is told once.
///
/// # Errors
///
/// A database error, or the notifier's.
pub async fn order_released(state: &AppState, order_id: i64) -> Result<i64> {
    let Some(order) = SalesOrder::find(&state.db, order_id).await? else {
        return Ok(0);
    };
    let mut total = 0;
    let mut served = Vec::new();
    for line in item_lines(state, &order).await? {
        let got = take(state, &order, &line, None).await?;
        if got > 0 {
            total += got;
            served.push((line.item_code.clone(), got));
        }
    }
    if total > 0 {
        tell(state, &order, &served).await?;
    }
    Ok(total)
}

/// A goods receipt was reversed: what was reserved from it is given back and
/// the lines are back-ordered again.
///
/// # Errors
///
/// A database error.
pub async fn on_reversed(state: &AppState, event: &StockArrivalReversed) -> Result {
    for arrived in &event.lines {
        let rows = BackOrderServe::query()
            .where_(BackOrderServe::SOURCE_TYPE.eq(event.source.subject_type.clone()))
            .where_(BackOrderServe::SOURCE_ID.eq(event.source.subject_id))
            .where_(BackOrderServe::ITEM_CODE.eq(arrived.item_code.clone()))
            .where_(BackOrderServe::REVERSED_AT.is_null())
            .get(&state.db)
            .await?;
        for row in rows {
            let Some(line) = SaleLine::find(&state.db, row.line_id).await? else {
                continue;
            };
            let back = row.quantity_milli.min(line.reserved_milli);
            if back > 0 {
                let owner = Subject::new(SALES_ORDER_SUBJECT_TYPE, row.order_id);
                stock_reservations(state)
                    .release(state, &owner, Some(line.id), Some(back))
                    .await?;
                let reserved = line.reserved_milli - back;
                let backordered = line.backordered_milli + back;
                SaleLine::query()
                    .where_(SaleLine::ID.eq(line.id))
                    .update(
                        &state.db,
                        &[
                            ("reserved_milli", &reserved),
                            ("backordered_milli", &backordered),
                        ],
                    )
                    .await?;
            }
            let at = Some(renox::db::now());
            BackOrderServe::query()
                .where_(BackOrderServe::ID.eq(row.id))
                .update(&state.db, &[("reversed_at", &at)])
                .await?;
        }
    }
    Ok(())
}

/// Takes `quantity` off line `line`: off the back-order first (a cancelled
/// back-order needs no outcome: nothing was picked), then off the
/// reservation, or the reverse when `back_order_first` is false (a
/// hand-over). The reservation part is given back to stock.
///
/// # Errors
///
/// A database error.
pub async fn reduce(
    state: &AppState,
    line: i64,
    quantity: i64,
    back_order_first: bool,
) -> Result {
    let Some(found) = SaleLine::find(&state.db, line).await? else {
        return Ok(());
    };
    if found.reserved_milli == 0 && found.backordered_milli == 0 {
        return Ok(());
    }
    let (from_back, from_reserved) = if back_order_first {
        let b = quantity.min(found.backordered_milli);
        (b, (quantity - b).min(found.reserved_milli))
    } else {
        let r = quantity.min(found.reserved_milli);
        ((quantity - r).min(found.backordered_milli), r)
    };
    if from_reserved > 0 {
        let owner = Subject::new(SALES_ORDER_SUBJECT_TYPE, found.subject_id);
        stock_reservations(state)
            .release(state, &owner, Some(found.id), Some(from_reserved))
            .await?;
    }
    let reserved = found.reserved_milli - from_reserved;
    let backordered = found.backordered_milli - from_back;
    SaleLine::query()
        .where_(SaleLine::ID.eq(found.id))
        .update(
            &state.db,
            &[
                ("reserved_milli", &reserved),
                ("backordered_milli", &backordered),
            ],
        )
        .await?;
    Ok(())
}

/// An order ended without being fulfilled: every reservation is given back
/// and every back-order dropped.
///
/// # Errors
///
/// A database error.
pub async fn on_ended(state: &AppState, order_id: i64) -> Result {
    let owner = Subject::new(SALES_ORDER_SUBJECT_TYPE, order_id);
    stock_reservations(state)
        .release(state, &owner, None, None)
        .await?;
    renox::db::sql(format!(
        "UPDATE {} SET reserved_milli = 0, backordered_milli = 0, updated_at = ? \
         WHERE subject_type = '{}' AND subject_id = ?",
        SaleLine::TABLE,
        SALES_ORDER_SUBJECT_TYPE
    ))
    .bind(renox::db::now())
    .bind(order_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// The open back-ordered quantity per item and location, over the orders
/// that are `confirmed`, `partially_fulfilled` or on `credit_hold`.
///
/// # Errors
///
/// A database error.
pub async fn open_demand(state: &AppState) -> Result<Vec<BackOrdered>> {
    let lines = SaleLine::query()
        .where_(SaleLine::SUBJECT_TYPE.eq(SALES_ORDER_SUBJECT_TYPE))
        .where_(SaleLine::BACKORDERED_MILLI.gt(0i64))
        .get(&state.db)
        .await?;
    let mut sums: BTreeMap<(String, Option<i64>), i64> = BTreeMap::new();
    for line in lines {
        let Some(order) = SalesOrder::find(&state.db, line.subject_id).await? else {
            continue;
        };
        if !matches!(
            order.status,
            SalesOrderStatus::Confirmed
                | SalesOrderStatus::PartiallyFulfilled
                | SalesOrderStatus::CreditHold
        ) {
            continue;
        }
        *sums
            .entry((line.item_code.clone(), order.location_id))
            .or_insert(0) += line.backordered_milli;
    }
    Ok(sums
        .into_iter()
        .map(|((item_code, location), quantity_milli)| BackOrdered {
            item_code,
            location: location.map(|id| Subject::new(LOCATION_SUBJECT_TYPE, id)),
            quantity_milli,
        })
        .collect())
}

/// The module's [`BackOrderDemand`].
pub(crate) struct Demand;

impl BackOrderDemand for Demand {
    fn open<'a>(&'a self, state: &'a AppState) -> BackOrderFuture<'a, Vec<BackOrdered>> {
        Box::pin(async move { open_demand(state).await })
    }
}

/// Registers the notification type, the demand and the listeners.
pub(crate) fn register(app: &mut Registry) {
    notify::register_type(notice_type());
    provide_back_orders(app, Demand);
    app.listen(
        |event: Transitioned<SalesOrder>, state: AppState| async move {
            match event.transition.as_str() {
                SUBMIT | ACCEPT | CONFIRM_PAYMENT => on_confirmed(&state, event.key).await,
                CANCEL | REJECT => on_ended(&state, event.key).await,
                FULFIL | FULFIL_LAST => {
                    let Ok(request) =
                        renox::serde_json::from_value::<FulfilOrder>(event.meta.clone())
                    else {
                        return Ok(());
                    };
                    for taken in &request.lines {
                        reduce(&state, taken.line, taken.quantity_milli, false).await?;
                    }
                    Ok(())
                }
                CANCEL_LINE | CANCEL_LINE_PARTLY => {
                    let Ok(input) =
                        renox::serde_json::from_value::<CancelLineInput>(event.meta.clone())
                    else {
                        return Ok(());
                    };
                    reduce(&state, input.line, input.quantity_milli, true).await
                }
                _ => Ok(()),
            }
        },
    );
    app.listen(|event: StockArrived, state: AppState| async move {
        for line in &event.lines {
            serve(&state, &event.location, &line.item_code, &event.source).await?;
        }
        Ok(())
    });
    app.listen(|event: StockArrivalReversed, state: AppState| async move {
        on_reversed(&state, &event).await
    });
}
