//! The accountant notice of a reopened supplier bill (wl-invoicing §4
//! `unallocate`, #75 A9).
//!
//! After the commit of `unallocate` or `unallocate_partially`
//! ([`crate::bill_events`]), each user with the `accountant` role (global
//! scope) gets `invoicing.supplier_bill_reopened` once: the supplier invoice
//! number, supplier, amount reopened, new balance, new state, the reason
//! (such as "bank return" or "debit note cancelled") and, for a reversed
//! payment, "pay again or fix the bank details".

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::invoicing::{BillReopenReason, SupplierBillReopened};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::bill_unallocate::reason_key;
use crate::supplier_bill::SupplierBill;
use crate::task::ACCOUNTANT;
use crate::texts::english;

/// The notification type of a reopened supplier bill.
pub const SUPPLIER_BILL_REOPENED: &str = "invoicing.supplier_bill_reopened";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "supplier_bills";

/// The reason in words: "payment reversed (bank return)". `text` maps a text
/// key to its text; a reversal reason without a text of its own is shown as
/// it is.
pub fn reason_text(
    text: impl Fn(&str) -> String,
    reason: BillReopenReason,
    reversal_reason: Option<&str>,
) -> String {
    let base = text(&format!(
        "wl_invoicing.reopen.reason.{}",
        reason_key(reason)
    ));
    let detail = reversal_reason.map(str::trim).filter(|r| !r.is_empty());
    match detail {
        Some(detail) => {
            let key = format!("wl_invoicing.reopen.reason.{detail}");
            let known = text(&key);
            let shown = if known == key {
                detail.to_owned()
            } else {
                known
            };
            format!("{base} ({shown})")
        }
        None => base,
    }
}

/// The text of a bill state.
fn status_text(status: wl_kernel::invoicing::SupplierBillStatus) -> String {
    let key = match renox::serde_json::to_value(status) {
        Ok(Value::String(name)) => format!("wl_invoicing.supplier_bill.status.{name}"),
        _ => String::new(),
    };
    english(&key).to_owned()
}

/// The values the notice texts are filled with.
pub fn data(
    bill: &SupplierBill,
    supplier: &str,
    event: &SupplierBillReopened,
    reversal_reason: Option<&str>,
) -> Value {
    let hint_key = format!(
        "wl_invoicing.supplier_bill_reopened.hint.{}",
        reason_key(event.reason)
    );
    json!({
        "number": bill.supplier_invoice_number,
        "supplier": supplier,
        "amount": amount_text(&bill.currency, event.amount),
        "balance": amount_text(&bill.currency, event.balance),
        "new_status": status_text(event.new_status),
        "reason": reason_text(|key| english(key).to_owned(), event.reason, reversal_reason),
        "hint": english(&hint_key),
        "link": format!("/supplier-bills/{}", bill.id),
    })
}

/// Tells each `accountant` that `bill` was reopened as `event` says: one
/// notice per user, by id, without repeats. Nothing when nobody holds the
/// role.
///
/// # Errors
///
/// A database or notifier error.
pub async fn notify(
    state: &AppState,
    bill: &SupplierBill,
    event: &SupplierBillReopened,
    reversal_reason: Option<&str>,
) -> Result {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT, &Scope::global())
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
        .billing_identity(&state.db, bill.supplier())
        .await?
        .map(|identity| identity.legal_name_en)
        .unwrap_or_default();
    let mut request = NotifyRequest::new(SUPPLIER_BILL_REOPENED, bill.subject()).data(data(
        bill,
        &supplier,
        event,
        reversal_reason,
    ));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The notification type [`SUPPLIER_BILL_REOPENED`]: staff, transactional,
/// by e-mail (and the bell).
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(SUPPLIER_BILL_REOPENED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.supplier_bill_reopened.subject")),
            english("wl_invoicing.supplier_bill_reopened.body"),
        )
}

/// Registers [`SUPPLIER_BILL_REOPENED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supplier_bill::SupplierBillStatus;
    use wl_kernel::PartyId;
    use wl_kernel::invoicing::{SupplierBillId, SupplierBillStatus as KernelStatus};

    fn event(reason: BillReopenReason) -> SupplierBillReopened {
        SupplierBillReopened {
            bill: SupplierBillId(3),
            supplier: PartyId(2),
            source: wl_kernel::Subject::new("payments.payment", 1),
            amount: 1_050_000,
            balance: 1_050_000,
            new_status: KernelStatus::Approved,
            reason,
        }
    }

    fn bill() -> SupplierBill {
        SupplierBill {
            id: 3,
            supplier_invoice_number: "INV-77".into(),
            currency: "AED".into(),
            status: SupplierBillStatus::Approved,
            ..Default::default()
        }
    }

    #[test]
    fn the_type_has_the_documented_key_and_a_transactional_email() {
        let def = notification_type();
        assert_eq!(def.key, "invoicing.supplier_bill_reopened");
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }

    #[test]
    fn a_reversed_payment_names_the_return_and_says_what_to_do() {
        let d = data(
            &bill(),
            "Blue Dunes",
            &event(BillReopenReason::PaymentReversed),
            Some("bank_return"),
        );
        assert_eq!(d["number"], "INV-77");
        assert_eq!(d["supplier"], "Blue Dunes");
        assert_eq!(d["new_status"], "Approved");
        assert_eq!(d["link"], "/supplier-bills/3");
        let reason = d["reason"].as_str().unwrap();
        assert!(reason.contains("bank return"), "{reason}");
        assert_eq!(d["hint"], "Pay again or fix the bank details.");
        assert!(d["amount"].as_str().unwrap().contains("10,500.00"));
    }

    #[test]
    fn a_cancelled_debit_note_has_its_own_reason_and_no_hint() {
        let d = data(
            &bill(),
            "Blue Dunes",
            &event(BillReopenReason::DebitNoteCancelled),
            None,
        );
        assert_eq!(d["reason"], "debit note cancelled");
        assert_eq!(d["hint"], "");
    }
}
