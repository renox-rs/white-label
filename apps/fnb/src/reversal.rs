//! Refunding a completed order whose delivery is reversed (06 F&B §4.2 row
//! *Customer says the order never arrived*, §7.1 *Order*, `WRK.ACL.10`,
//! `SAL.ACL.06`; BL-FNB-07).
//!
//! A customer says the delivered order never arrived. The kitchen manager
//! puts the delivery under review (wl-delivery `open_review`, the order stays
//! `completed`) and checks the proof. If the rider was wrong, a kitchen
//! manager reverses the delivery (`reverse_delivery`, approved by a second
//! one; the rider can neither request nor approve it, `WRK.ACL.10`). If the
//! review is upheld, nothing changes.
//!
//! On `DeliveryReversed` for an F&B order ([`on_reversed`]) the system raises
//! a sales return of kind *no goods* on every line the customer received, the
//! category *other* ("not delivered (delivery reversed)") and the party the
//! reversal blamed (*driver or rider* when the liability is the driver's), and
//! resolves it at once, with no second approval (the kitchen manager's
//! reversal approval counts as the `SAL.ACL.06` approval):
//!
//! - **refund** for a prepaid order, or cash on delivery whose cash was handed
//!   over and kept: wl-payments is asked for the refund and the cashier
//!   records the method when it is paid;
//! - **credit note only** for cash on delivery that nobody handed over (the
//!   cash left the rider's expected cash with the reversal) or that the
//!   reversal's approver already refunded to the payer: no refund payment is
//!   asked, so nothing is paid twice, and the amount due is cleared.
//!
//! The order is never reopened or cancelled; its amount refunded becomes the
//! full amount. Once is enough: a second `DeliveryReversed` for the same order
//! (a re-delivery reversed again) raises nothing.

use renox::prelude::*;
use wl_kernel::delivery::DeliveryReversed;
use wl_kernel::sales::{NoGoodsRefund, order_desk};

use crate::fulfilment::order_of_delivery;
use crate::notices::{about, tell_customer, tell_managers};
use crate::money::aed;
use crate::order::{FnbOrder, FnbOrderStatus};
use crate::permissions;
use crate::texts::english;

/// The complaint category of the return (a stored name of the sales
/// settings).
pub const CATEGORY: &str = "other";

/// The party of the sales settings that stands for the rider.
pub const RIDER_PARTY: &str = "driver_or_rider";

/// How the return was resolved: a refund.
pub const REFUND: &str = "refund";

/// How the return was resolved: by credit note only.
pub const CREDIT_NOTE: &str = "credit_note";

/// The sales settings' party for the liability a reversal names (`driver`,
/// `business`, `client`, `none`).
pub fn party_of(liability: &str) -> &'static str {
    match liability {
        "business" | "none" => "business",
        "client" => "customer",
        _ => RIDER_PARTY,
    }
}

/// Whether the return is resolved by credit note only: cash on delivery whose
/// cash was not handed over (taken out of the rider's custody) or was already
/// refunded to the payer by the reversal; a prepaid order, or cash that was
/// kept, is refunded.
pub fn credit_only(order: &FnbOrder, cash_treatment: Option<&str>) -> bool {
    order.is_cash_on_delivery() && cash_treatment != Some("kept")
}

/// `DeliveryReversed`: the completed order of the delivery is refunded in
/// full by a *no goods* return, once.
///
/// # Errors
///
/// Database errors. A refusal of the sales module is told to the kitchen
/// managers instead (nothing is lost: the return can be raised by hand).
pub(crate) async fn on_reversed(state: &AppState, event: DeliveryReversed) -> Result {
    let Some(order) = order_of_delivery(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    if order.status != FnbOrderStatus::Completed {
        eprintln!(
            "fnb: delivery {} was reversed but order {} is {}: no refund",
            event.delivery,
            order.number,
            order.status.as_str()
        );
        return Ok(());
    }
    if order.reversal_return_id.is_some() {
        return Ok(());
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    // Every line the customer received, in full.
    let lines: Vec<(i64, i64)> = view
        .lines
        .iter()
        .filter(|line| line.is_item && line.fulfilled_milli > 0 && line.quantity_milli > 0)
        .map(|line| {
            let worth = i128::from(line.gross) * i128::from(line.fulfilled_milli)
                / i128::from(line.quantity_milli);
            (line.id, i64::try_from(worth).unwrap_or(line.gross))
        })
        .collect();
    let amount: i64 = lines.iter().map(|(_, fils)| fils).sum();
    let credit = credit_only(&order, event.cash_treatment.as_deref());
    let party = party_of(&event.liability);
    let request = NoGoodsRefund {
        order: order.sales_order(),
        lines,
        category: CATEGORY.to_owned(),
        party: party.to_owned(),
        reason: english("fnb.reversal.reason", &[]),
        credit_only: credit,
    };
    match order_desk(state).refund_no_goods(state, request).await {
        Ok(return_id) => {
            let linked = Some(return_id);
            let at = Some(renox::db::now());
            let how = Some(if credit { CREDIT_NOTE } else { REFUND }.to_owned());
            let whose = Some(party.to_owned());
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(order.id))
                .update(
                    &state.db,
                    &[
                        ("reversal_return_id", &linked),
                        ("reversed_at", &at),
                        ("reversal_resolution", &how),
                        ("reversal_amount", &amount),
                        ("reversal_party", &whose),
                    ],
                )
                .await?;
            tell_customer(
                state,
                &order,
                &about("fnb.reversal.customer_title", &order),
                &english(
                    if credit {
                        "fnb.reversal.customer_body_credit"
                    } else {
                        "fnb.reversal.customer_body_refund"
                    },
                    &[("number", &order.number), ("amount", &aed(amount))],
                ),
            )
            .await?;
            if !credit {
                // The cashier records how the refund is paid.
                tell_managers(
                    state,
                    &order,
                    permissions::ORDERS_MANAGE,
                    &about("fnb.reversal.refund_title", &order),
                    &english(
                        "fnb.reversal.refund_body",
                        &[("number", &order.number), ("amount", &aed(amount))],
                    ),
                )
                .await?;
            }
        }
        Err(error) => {
            let problem = error.to_string();
            eprintln!(
                "fnb: no refund was raised for the reversed delivery of {}: {problem}",
                order.number
            );
            tell_managers(
                state,
                &order,
                permissions::ORDERS_MANAGE,
                &about("fnb.reversal.failed_title", &order),
                &english(
                    "fnb.reversal.failed_body",
                    &[("number", &order.number), ("problem", &problem)],
                ),
            )
            .await?;
        }
    }
    Ok(())
}
