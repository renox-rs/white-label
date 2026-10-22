//! A line the customer refuses at the door (06 F&B §4.2 row *Customer refuses
//! one item at the door*, §7.2 *Partial acceptance*, `SAL.ACL.06`; BL-FNB-16).
//!
//! Partial acceptance is on for the F&B order (a setting of the host, never
//! decided here): the rider records the refused line with the refusal reason
//! and the delivery is `delivered`; the order `completes` for the accepted
//! lines. On `DeliveryDelivered` with a refused line ([`after_delivered`]):
//!
//! - **prepaid order**: the system raises a sales return of kind *no goods* on
//!   the refused quantity (category *wrong item*, party *business*) and
//!   resolves it at once with a *refund* up to the limit
//!   ([`FnbSettings::refused_refund_limit`](crate::settings::FnbSettings),
//!   AED 50, the `SAL.ACL.06` limit of the sales settings). Above it nothing
//!   is refunded by the system: the kitchen managers are told the amount that
//!   waits and a kitchen manager files the refund on the order, where the
//!   sales module asks for the approval of a holder other than the person who
//!   recorded it;
//! - **cash on delivery**: the delivery expects the accepted lines' amount
//!   only (the delivery request carries each line's amount), so nothing was
//!   collected for the refused line and nothing is refunded.
//!
//! The refused food that comes back on the run is posted as wastage by the
//! stock module on `DeliveryRefusedReceived`: that posting by sale line is
//! not offered by the stock module yet (`DEFERRED-fnb-settings.md`).

use renox::prelude::*;
use wl_kernel::delivery::{DeliveredLine, DeliveryDelivered};
use wl_kernel::sales::{NoGoodsRefund, order_desk};

use crate::fulfilment::SALE_LINE_SUBJECT_TYPE;
use crate::money::aed;
use crate::notices::{about, tell_managers};
use crate::order::FnbOrder;
use crate::permissions;
use crate::settings::settings;
use crate::texts::english;

/// The complaint category of a refused line (a name of the sales settings).
pub const CATEGORY: &str = "wrong_item";

/// The responsible party of a refused line (a name of the sales settings).
pub const PARTY: &str = "business";

/// The refused lines of `lines` with the sale line and the value refused, in
/// fils, of each (the line's value in proportion to the refused quantity).
fn refused_value(lines: &[DeliveredLine], view: &wl_kernel::sales::OrderView) -> Vec<(i64, i64)> {
    let mut refused = Vec::new();
    for line in lines.iter().filter(|l| l.refused_milli > 0) {
        let Some(source) = line.line.as_ref() else {
            continue;
        };
        if source.subject_type != SALE_LINE_SUBJECT_TYPE {
            continue;
        }
        let Some(sale) = view.lines.iter().find(|l| l.id == source.subject_id) else {
            continue;
        };
        if sale.quantity_milli <= 0 {
            continue;
        }
        let worth = i128::from(sale.gross) * i128::from(line.refused_milli)
            / i128::from(sale.quantity_milli);
        refused.push((sale.id, i64::try_from(worth).unwrap_or(sale.gross)));
    }
    refused
}

/// After `DeliveryDelivered` completed `order`: refunds what the customer
/// refused at the door, as the module comment says.
///
/// # Errors
///
/// Database errors and the notifier's. A refund the sales module refuses is
/// told to the kitchen managers, not raised as an error: the delivery stays
/// delivered.
pub(crate) async fn after_delivered(
    state: &AppState,
    order: &FnbOrder,
    event: &DeliveryDelivered,
) -> Result {
    if !event.lines.iter().any(|l| l.refused_milli > 0) {
        return Ok(());
    }
    // Cash on delivery collected the accepted lines only: nothing to refund.
    if order.is_cash_on_delivery() {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    let lines = refused_value(&event.lines, &view);
    let amount: i64 = lines.iter().map(|(_, fils)| fils).sum();
    if lines.is_empty() || amount <= 0 {
        return Ok(());
    }
    if amount > settings(state).refused_refund_limit {
        tell_managers(
            state,
            order,
            permissions::ORDERS_MANAGE,
            &about("fnb.refused.waiting_title", order),
            &english(
                "fnb.refused.waiting_body",
                &[("number", &order.number), ("amount", &aed(amount))],
            ),
        )
        .await?;
        return Ok(());
    }
    let request = NoGoodsRefund {
        order: order.sales_order(),
        lines,
        category: CATEGORY.to_owned(),
        party: PARTY.to_owned(),
        reason: english("fnb.refused.reason", &[]),
        credit_only: false,
    };
    if let Err(error) = order_desk(state).refund_no_goods(state, request).await {
        tell_managers(
            state,
            order,
            permissions::ORDERS_MANAGE,
            &about("fnb.refused.failed_title", order),
            &english(
                "fnb.refused.failed_body",
                &[("number", &order.number), ("problem", &error.to_string())],
            ),
        )
        .await?;
    }
    Ok(())
}
