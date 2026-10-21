//! The notices of the payer bill (wl-invoicing §4 *Payer bill*
//! `transfer_to_customer` and `void`).
//!
//! - **`invoicing.payer_bill_transferred`**: the customer is told that what
//!   the payer will not pay is now theirs to pay.
//! - **`invoicing.payer_bill_voided`**: the billing coordinator is told that
//!   a payer bill was voided, so the source can be split again to the right
//!   payer.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::config_of;
use crate::payer_bill::PayerBill;
use crate::texts::english;

/// The notification type of what moved to the customer.
pub const TRANSFERRED: &str = "invoicing.payer_bill_transferred";
/// The notification type of a voided payer bill.
pub const VOIDED: &str = "invoicing.payer_bill_voided";

/// Registers [`TRANSFERRED`] and [`VOIDED`] with the kernel registry.
pub(crate) fn register_types() {
    for (key, group, subject, body) in [
        (
            TRANSFERRED,
            "payer_bills",
            "wl_invoicing.payer_bill_transferred.subject",
            "wl_invoicing.payer_bill_transferred.body",
        ),
        (
            VOIDED,
            "payer_bills",
            "wl_invoicing.payer_bill_voided.subject",
            "wl_invoicing.payer_bill_voided.body",
        ),
    ] {
        notify::register_type(
            NotificationTypeDef::new(key, "invoicing")
                .group(group)
                .category(Category::Transactional)
                .template(
                    NotifyChannel::Email,
                    "en",
                    Some(english(subject)),
                    english(body),
                ),
        );
    }
}

/// Tells the customer of `bill` that `amount` fils moved to them.
pub(crate) async fn tell_customer(state: &AppState, bill: &PayerBill, amount: i64) -> Result {
    let payer = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, bill.payer())
        .await?
        .map(|identity| identity.legal_name_en)
        .unwrap_or_default();
    let request = NotifyRequest::new(TRANSFERRED, bill.subject())
        .to_party(bill.customer())
        .data(json!({
            "number": bill.number.clone().unwrap_or_default(),
            "payer": payer,
            "amount": amount_text(&bill.currency, amount),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the billing coordinators that `bill` was voided.
pub(crate) async fn tell_coordinators(state: &AppState, bill: &PayerBill) -> Result {
    let role = config_of(state).payer_bill_task_role.clone();
    let mut ids: Vec<i64> = users_with_role_in(&state.db, &role, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(VOIDED, bill.subject()).data(json!({
        "number": bill.number.clone().unwrap_or_else(|| format!("#{}", bill.id)),
        "source": bill.source().to_string(),
        "link": format!("/payer-bills/{}", bill.id),
    }));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
