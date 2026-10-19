//! The notice of a reopened invoice (wl-invoicing §4 `unallocate`, #70 A8).
//!
//! After the commit of `unallocate`, `unallocate_partially` or
//! `unallocate_overdue` ([`crate::events`]), the invoice's owner
//! (`owner_id`, when that user still exists) and each holder of the
//! `receivables` role (global scope) get `invoicing.invoice_reopened`: the
//! number, party, amount reopened, new balance, new state and the reason
//! (such as "bank return"). Nobody is told twice; without an owner only the
//! role is told.
//!
//! A reversed recovery also tells the accountants (the `relief_notify_role`)
//! with `invoicing.recovery_reversed`: the amount, the recovered total now
//! and what happened on the VAT side ([`notify_recovery_reversed`]).

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::invoicing::{InvoiceReopened, InvoiceStatus, ReopenReason};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::allocate::amount_text;
use crate::config_of;
use crate::invoice::Invoice;
use crate::recovery::Recovery;
use crate::texts::english;
use crate::unallocate::reason_key;

/// The notification type of a reopened invoice.
pub const INVOICE_REOPENED: &str = "invoicing.invoice_reopened";

/// The notification type of a reversed recovery.
pub const RECOVERY_REVERSED: &str = "invoicing.recovery_reversed";

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

/// The VAT effect of a reversal in words, from the recovery's
/// `reversed_effect` and VAT share.
pub fn vat_effect_text(text: impl Fn(&str) -> String, recovery: &Recovery) -> String {
    let effect = recovery.reversed_effect.as_deref().unwrap_or("none");
    let base = text(&format!("wl_invoicing.recovery_reversed.effect.{effect}"));
    format!(
        "{base} (VAT share {})",
        amount_text("AED", recovery.vat_share)
    )
}

/// The values the accountant's notice is filled with.
pub fn recovery_data(invoice: &Invoice, recovery: &Recovery) -> Value {
    let party_name = invoice
        .party_snapshot
        .as_ref()
        .map(|snapshot| snapshot.0.legal_name_en.clone())
        .unwrap_or_default();
    json!({
        "number": invoice.number.clone().unwrap_or_default(),
        "party_name": party_name,
        "amount": amount_text(&invoice.currency, recovery.amount),
        "recovered_total": amount_text(&invoice.currency, invoice.write_off_recovered_amount),
        "written_off": amount_text(&invoice.currency, invoice.write_off_amount.unwrap_or_default()),
        "vat_effect": vat_effect_text(|key| english(key).to_owned(), recovery),
        "link": format!("/invoices/{}", invoice.id),
    })
}

/// Tells each holder of the notify role that recovery `recovery_id` of
/// `invoice` was reversed: one notice per user. Nothing when the recovery
/// is not found.
///
/// # Errors
///
/// A database or notifier error.
pub async fn notify_recovery_reversed(
    state: &AppState,
    invoice: &Invoice,
    recovery_id: i64,
) -> Result {
    let Some(recovery) = Recovery::find(&state.db, recovery_id).await? else {
        return Ok(());
    };
    let config = config_of(state);
    let mut ids: Vec<i64> =
        users_with_role_in(&state.db, &config.relief_notify_role, &Scope::global())
            .await?
            .iter()
            .map(|user| user.id)
            .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request = NotifyRequest::new(RECOVERY_REVERSED, invoice.subject())
        .data(recovery_data(invoice, &recovery));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The notification type [`RECOVERY_REVERSED`]: staff, transactional, by
/// e-mail (and the bell).
pub fn recovery_notification_type() -> NotificationTypeDef {
    NotificationTypeDef::new(RECOVERY_REVERSED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_invoicing.recovery_reversed.subject")),
            english("wl_invoicing.recovery_reversed.body"),
        )
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
    notify::register_type(recovery_notification_type());
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
    fn the_accountant_notice_names_the_vat_effect() {
        let invoice = Invoice {
            id: 4,
            number: Some("INV-PGP-2026-0004".into()),
            currency: "AED".into(),
            write_off_amount: Some(105_000),
            write_off_recovered_amount: 0,
            ..Default::default()
        };
        let recovery = Recovery {
            amount: 105_000,
            vat_share: 5_000,
            reversed_effect: Some("relief_created".into()),
            ..Default::default()
        };
        let filled = recovery_data(&invoice, &recovery);
        assert_eq!(filled["amount"], "AED 1,050.00");
        assert_eq!(filled["recovered_total"], "AED 0.00");
        assert_eq!(filled["written_off"], "AED 1,050.00");
        let effect = filled["vat_effect"].as_str().unwrap();
        assert!(effect.contains("New bad-debt relief"), "{effect}");
        assert!(effect.contains("AED 50.00"), "{effect}");
        let def = recovery_notification_type();
        assert_eq!(def.key, RECOVERY_REVERSED);
        assert_eq!(def.category, Category::Transactional);
        assert!(
            def.template_for(NotifyChannel::Email, "en")
                .expect("template")
                .body
                .contains("{{ vat_effect }}")
        );
    }

    #[test]
    fn the_type_is_transactional_with_an_english_email() {
        let def = notification_type();
        assert_eq!(def.key, INVOICE_REOPENED);
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
    }
}
