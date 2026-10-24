//! The notices of the changes to a placed order (BL-SAL-05, BL-SAL-06,
//! BL-SAL-08, BL-SAL-09): the revised confirmation, the payment link of an
//! increase and the cancelled lines. (The credit hold notice, `SAL.RUL.01`,
//! is one for every hold: [`crate::notices::order_credit_held`].)
//! Transactional, English first, through the kernel notifier.

use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::amend::AmendDetail;
use crate::cancel_lines::CancelDetail;
use crate::notices::aed;
use crate::sales_order::SalesOrder;
use crate::texts::english;

/// The customer is told that the order has a new revision.
pub const ORDER_REVISED: &str = "sales.order_revised";
/// The customer is asked to pay the difference of an increase.
pub const ORDER_AMEND_LINK: &str = "sales.order_amend_link";
/// The customer is told that lines of the order were cancelled.
pub const ORDER_LINES_CANCELLED: &str = "sales.order_lines_cancelled";

/// A transactional e-mail type with English texts.
fn english_type(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("wl_sales.notices.{texts}_subject"), &[]);
    let body = english(&format!("wl_sales.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "sales")
        .group("sales")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_type(ORDER_REVISED, "order_revised"));
    notify::register_type(english_type(ORDER_AMEND_LINK, "order_amend_link"));
    notify::register_type(english_type(ORDER_LINES_CANCELLED, "order_lines_cancelled"));
}

/// Tells the customer of `order` about the revision now in force.
pub(crate) async fn order_revised(
    state: &AppState,
    order: &SalesOrder,
    detail: &AmendDetail,
) -> Result {
    let request = NotifyRequest::new(ORDER_REVISED, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "revision": detail.revision_after,
            "before": aed(detail.value_before),
            "total": aed(detail.value_after),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Asks the customer of `order` to pay `amount` fils on the page `path`.
pub(crate) async fn amend_link(
    state: &AppState,
    order: &SalesOrder,
    amount: i64,
    path: &str,
) -> Result {
    let request = NotifyRequest::new(ORDER_AMEND_LINK, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "amount": aed(amount),
            "link": path,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the customer of `order` that the lines in `detail` were cancelled.
pub(crate) async fn lines_cancelled(
    state: &AppState,
    order: &SalesOrder,
    detail: &CancelDetail,
) -> Result {
    let cancelled: i64 = detail.lines.iter().map(|l| l.share).sum();
    let fees: i64 = detail.lines.iter().map(|l| l.fee).sum();
    let request = NotifyRequest::new(ORDER_LINES_CANCELLED, order.subject())
        .to_party(order.party())
        .subject_label(&order.number)
        .data(json!({
            "number": order.number,
            "lines": detail.lines.len(),
            "value": aed(cancelled),
            "fee": aed(fees),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
