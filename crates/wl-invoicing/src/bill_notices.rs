//! The accountant notice of a held supplier bill (wl-invoicing §7,
//! `invoicing.supplier_bill_possible_duplicate`): after the commit of
//! `submit_held`, every user holding the notice role
//! ([`crate::InvoicingConfig::duplicate_notify_role`], global scope) gets one
//! message with both bills' supplier invoice numbers, the supplier, the
//! amount and a link to the held bill. Without wl-notify the kernel puts it
//! in the bell.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::config_of;
use crate::supplier_bill::SupplierBill;
use crate::texts::english;

/// The notification type of a held possible duplicate.
pub const POSSIBLE_DUPLICATE: &str = "invoicing.supplier_bill_possible_duplicate";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "supplier_bills";

/// The notification type: staff, transactional, by e-mail (the bell is the
/// kernel fallback).
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(POSSIBLE_DUPLICATE, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.supplier_bill_duplicate.subject")),
            english("wl_invoicing.supplier_bill_duplicate.body"),
        )
}

/// Registers [`POSSIBLE_DUPLICATE`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

/// The template variables of the notice for `held`, which may duplicate
/// `earlier`, from `supplier` (the name).
pub(crate) fn data(held: &SupplierBill, earlier: &SupplierBill, supplier: &str) -> Value {
    json!({
        "number": held.supplier_invoice_number,
        "earlier_number": earlier.supplier_invoice_number,
        "supplier": supplier,
        "amount": amount_text(&held.currency, held.gross_total),
        "link": format!("/supplier-bills/{}", held.id),
    })
}

/// Tells each holder of the notify role that bill `bill_id` is held, one
/// message per user. Nothing when the bill has no earlier bill or nobody
/// holds the role.
pub(crate) async fn notify_held(state: &AppState, bill_id: i64) -> Result {
    let Some(held) = SupplierBill::find(&state.db, bill_id).await? else {
        return Ok(());
    };
    let Some(earlier_id) = held.duplicate_of else {
        return Ok(());
    };
    let Some(earlier) = SupplierBill::find(&state.db, earlier_id).await? else {
        return Ok(());
    };
    let config = config_of(state);
    let mut ids: Vec<i64> =
        users_with_role_in(&state.db, &config.duplicate_notify_role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let supplier = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, held.supplier())
        .await?
        .map(|identity| identity.legal_name_en)
        .unwrap_or_default();
    let mut request = NotifyRequest::new(POSSIBLE_DUPLICATE, held.subject())
        .data(data(&held, &earlier, &supplier));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_type_has_the_documented_key() {
        assert_eq!(
            notification_type().key,
            "invoicing.supplier_bill_possible_duplicate"
        );
    }

    #[test]
    fn the_notice_names_both_numbers_the_supplier_and_a_link() {
        let held = SupplierBill {
            id: 2,
            supplier_invoice_number: "inv-77 ".into(),
            currency: "AED".into(),
            gross_total: 105_000,
            ..Default::default()
        };
        let earlier = SupplierBill {
            id: 1,
            supplier_invoice_number: "INV-77".into(),
            ..Default::default()
        };
        let d = data(&held, &earlier, "Blue Dunes");
        assert_eq!(d["number"], "inv-77 ");
        assert_eq!(d["earlier_number"], "INV-77");
        assert_eq!(d["supplier"], "Blue Dunes");
        assert_eq!(d["link"], "/supplier-bills/2");
        assert!(d["amount"].as_str().unwrap().contains("1,050"));
    }
}
