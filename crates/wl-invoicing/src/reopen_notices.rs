//! The notice of a reopened invoice (wl-invoicing §4 `unallocate`, #70 A8).
//!
//! After the commit of `unallocate`, `unallocate_partially` or
//! `unallocate_overdue` ([`crate::events`]), the invoice's owner
//! (`owner_id`, when that user still exists) and each holder of the
//! `receivables` role (global scope) get `invoicing.invoice_reopened`: the
//! number, party, amount reopened, new balance, new state and the reason
//! (such as "bank return"). Nobody is told twice; without an owner only the
//! role is told.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::invoicing::{InvoiceReopened, InvoiceStatus, ReopenReason};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::invoice::Invoice;
use crate::texts::english;
use crate::unallocate::reason_key;

/// The notification type of a reopened invoice.
pub const INVOICE_REOPENED: &str = "invoicing.invoice_reopened";

/// The role told besides the owner.
pub const RECEIVABLES: &str = "receivables";

/// The module that sends the notice.
const MODULE: &str = "invoicing";

/// The row of the wl-notify §5.4 default table the type belongs to.
const GROUP: &str = "invoices";

/// The reason in words: "payment reversed (bank return)". `text` maps a
/// text key to its text; a reversal reason without a text of its own is
/// shown as it is.
pub fn reason_text(
    text: impl Fn(&str) -> String,
    reason: ReopenReason,
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

/// The text of an invoice state.
fn status_text(status: InvoiceStatus) -> String {
    let key = match renox::serde_json::to_value(status) {
        Ok(Value::String(name)) => format!("wl_invoicing.status.{name}"),
        _ => String::new(),
    };
    english(&key).to_owned()
}

/// The values the notice texts are filled with.
pub fn data(invoice: &Invoice, event: &InvoiceReopened) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text(&invoice.currency, event.amount),
        "balance": amount_text(&invoice.currency, event.balance),
        "new_status": status_text(event.new_status),
        "reason": reason_text(
            |key| english(key).to_owned(),
            event.reason,
            event.reversal_reason.as_deref(),
        ),
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// The users to tell: the owner when they exist, then the `receivables`
/// holders, sorted and without repeats.
pub async fn recipients(db: &Db, invoice: &Invoice) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    if let Some(owner_id) = invoice.owner_id
        && User::find(db, owner_id).await?.is_some()
    {
        ids.push(owner_id);
    }
    let mut holders: Vec<i64> = users_with_role_in(db, RECEIVABLES, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    holders.sort_unstable();
    for id in holders {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Tells the owner and the `receivables` role that `invoice` was reopened
/// as `event` says: one notice each.
///
/// # Errors
///
/// A database or notifier error.
pub async fn notify(state: &AppState, invoice: &Invoice, event: &InvoiceReopened) -> Result {
    let data = data(invoice, event);
    let notifier = notify::notifier(state);
    for id in recipients(&state.db, invoice).await? {
        let notice = NotifyRequest::new(INVOICE_REOPENED, invoice.subject())
            .to_user(id)
            .data(data.clone());
        notifier.notify(state, notice).await?;
    }
    Ok(())
}

/// The notification type [`INVOICE_REOPENED`]: staff, transactional, by
/// e-mail (and the bell).
pub fn notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(INVOICE_REOPENED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.reopened.subject")),
            english("wl_invoicing.reopened.body"),
        )
}

/// Registers [`INVOICE_REOPENED`] with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(notification_type());
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyId;
    use wl_kernel::Subject;
    use wl_kernel::invoicing::InvoiceId;

    fn english_text(key: &str) -> String {
        english(key).to_owned()
    }

    #[test]
    fn the_reason_names_the_payments_reversal() {
        let text = reason_text(
            english_text,
            ReopenReason::PaymentReversed,
            Some("bank_return"),
        );
        assert_eq!(text, "payment reversed (bank return)");
        assert_eq!(
            reason_text(english_text, ReopenReason::PaymentReversed, Some("odd")),
            "payment reversed (odd)"
        );
        assert_eq!(
            reason_text(english_text, ReopenReason::AllocationMoved, None),
            "allocation moved"
        );
        assert_eq!(
            reason_text(english_text, ReopenReason::RecoveryReversed, Some(" ")),
            "recovery reversed"
        );
    }

    #[test]
    fn the_notice_carries_the_figures_and_the_state() {
        let invoice = Invoice {
            id: 4,
            number: Some("INV-PGP-2026-0004".into()),
            currency: "AED".into(),
            ..Default::default()
        };
        let event = InvoiceReopened {
            invoice: InvoiceId(4),
            party: PartyId(7),
            source: Subject::new("property.lease", 9),
            money_source: Subject::new("payments.payment", 3),
            amount: 200_000,
            balance: 200_000,
            new_status: InvoiceStatus::PartiallyPaid,
            reason: ReopenReason::PaymentReversed,
            reversal_reason: Some("bank_return".into()),
        };
        let filled = data(&invoice, &event);
        assert_eq!(filled["number"], "INV-PGP-2026-0004");
        assert_eq!(filled["amount"], "AED 2,000.00");
        assert_eq!(filled["balance"], "AED 2,000.00");
        assert_eq!(filled["new_status"], "Partially paid");
        assert!(filled["reason"].as_str().unwrap().contains("bank return"));
        assert_eq!(filled["link"], "/invoices/4");
    }

    #[test]
    fn the_type_is_transactional_with_an_english_email() {
        let def = notification_type();
        assert_eq!(def.key, INVOICE_REOPENED);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
