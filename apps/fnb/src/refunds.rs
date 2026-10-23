//! The manager-approved refund of a failure on the customer's side
//! (06 F&B §3.3 `FNB.ACL.17`; BL-FNB-02): for a customer-side reason the
//! prepaid order is closed with *no refund*, and the cashier may ask for one
//! from the cancelled order within 7 days of the cancellation, full or
//! partial, up to the amount paid. The refund is a wl-payments refund the
//! kitchen manager approves (`refunds.approve`); the requester never approves
//! it (`FIN.ACL.04`, which wl-payments enforces).
//!
//! The 7 days run from the moment the order became `cancelled`
//! (`closed_at`): a request exactly 7 days after is accepted, a later one is
//! refused with "refund request window closed". The days are a setting
//! ([`crate::settings::FnbSettings::refund_request_days`]).

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::payments::RefundApproved;
use wl_kernel::sales::order_desk;

use crate::close::request_refund;
use crate::order::{FnbOrder, FnbOrderStatus, decision};
use crate::permissions;
use crate::placing::refused;
use crate::settings::settings;
use crate::texts::english;

/// A refund a cashier asked for on a cancelled order.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_refund_requests", index(order_id))]
pub struct RefundAsk {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "fnb_orders")]
    pub order_id: i64,
    /// The amount asked, in fils.
    pub amount: i64,
    /// Why.
    pub reason: String,
    /// The cashier.
    pub requested_by: Option<i64>,
    /// The wl-payments refund, approved by someone other than the requester.
    pub refund_id: Option<i64>,
    /// The kitchen manager who approved it.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RefundAsk {
    /// The refunds asked for the order `order_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_order(db: &Db, order_id: i64) -> Result<Vec<RefundAsk>> {
        RefundAsk::query()
            .where_(RefundAsk::ORDER_ID.eq(order_id))
            .order_by(RefundAsk::ID)
            .get(db)
            .await
    }
}

/// The last moment a refund may be asked for `order`, or `None` when the
/// order was not closed after a failed delivery.
pub fn window_ends(order: &FnbOrder, days: i64) -> Option<DateTime> {
    order.closed_at.map(|closed| closed + TimeDelta::days(days))
}

/// What may still be asked for `order`, in fils: what was paid less the
/// refunds asked so far.
///
/// # Errors
///
/// Database errors.
pub async fn refundable(db: &Db, order: &FnbOrder) -> Result<i64> {
    let asked: i64 = RefundAsk::of_order(db, order.id)
        .await?
        .iter()
        .map(|ask| ask.amount)
        .sum();
    Ok((order.paid_amount - asked).max(0))
}

/// Why a refund cannot be asked for `order` now, in words; `None` when it
/// can. The page shows it beside the disabled button.
///
/// # Errors
///
/// Database errors.
pub async fn problem(state: &AppState, order: &FnbOrder) -> Result<Option<String>> {
    if order.status != FnbOrderStatus::Cancelled || order.closed_at.is_none() {
        return Ok(Some(english("fnb.refund.not_closed", &[])));
    }
    let decided = order.refund_decision.as_deref();
    if decided != Some(decision::NO_REFUND) && decided != Some(decision::MANAGER_APPROVED) {
        return Ok(Some(english("fnb.refund.no_need", &[])));
    }
    if let Some(end) = window_ends(order, settings(state).refund_request_days)
        && renox::db::now() > end
    {
        return Ok(Some(english("fnb.refund.window_closed", &[])));
    }
    if refundable(&state.db, order).await? <= 0 {
        return Ok(Some(english("fnb.refund.nothing_left", &[])));
    }
    Ok(None)
}

/// The cashier (`orders.refund_request`) asks for a refund of `amount` fils on
/// the cancelled order `id`.
///
/// # Errors
///
/// `Forbidden` without `orders.refund_request`; 422 for an order that was not
/// closed after a failed delivery with *no refund*, a request after the
/// window, an amount that is not above zero or above what is left of what was
/// paid, or no reason.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    amount: i64,
    reason: &str,
) -> Result<RefundAsk> {
    if !user.has_permission(permissions::ORDERS_REFUND_REQUEST) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    if let Some(text) = problem(state, &order).await? {
        return Err(refused(text));
    }
    let reason = reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(english("fnb.refund.reason_needed", &[])));
    }
    let left = refundable(&state.db, &order).await?;
    if amount <= 0 || amount > left {
        return Err(refused(english("fnb.refund.amount", &[])));
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Err(Error::NotFound);
    };
    let refund = request_refund(state, &order, &view, amount, &reason, Some(user.id)).await?;
    let mut ask = RefundAsk {
        order_id: order.id,
        amount,
        reason,
        requested_by: Some(user.id),
        refund_id: Some(refund),
        ..RefundAsk::default()
    };
    ask.insert(&state.db).await?;
    Ok(ask)
}

/// `RefundApproved`: the kitchen manager approved a refund a cashier asked
/// for; the order's decision is *manager approved* and the amount is added to
/// what it refunded.
pub(crate) async fn on_refund_approved(state: &AppState, event: RefundApproved) -> Result {
    let Some(ask) = RefundAsk::query()
        .where_(RefundAsk::REFUND_ID.eq(Some(event.refund.0)))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    if ask.approved_at.is_some() {
        return Ok(());
    }
    let (by, at) = (event.approved_by, Some(renox::db::now()));
    RefundAsk::query()
        .where_(RefundAsk::ID.eq(ask.id))
        .update(&state.db, &[("approved_by", &by), ("approved_at", &at)])
        .await?;
    let Some(order) = FnbOrder::find(&state.db, ask.order_id).await? else {
        return Ok(());
    };
    let decided = Some(decision::MANAGER_APPROVED.to_owned());
    let total = order.refund_amount + ask.amount;
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(
            &state.db,
            &[("refund_decision", &decided), ("refund_amount", &total)],
        )
        .await?;
    Ok(())
}
