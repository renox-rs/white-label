//! The notices the module sends through the kernel's `Notifier`
//! (wl-payments A9, A10).
//!
//! - `payments.payment_returned`: the party is told its payment was
//!   returned by the bank (transactional, e-mail, EN). Nothing is sent for
//!   `entered_in_error`.
//! - `payments.followup_task`: the holders of the accountant role get a
//!   notice for a new follow-up task.
//!
//! While `notify` is off the kernel's `NoNotifier` sends nothing to a party
//! and a bell notice to each user.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, money_text};
use crate::payment::Payment;
use crate::texts::english;

/// The type of the party's bank-return notice.
pub const PAYMENT_RETURNED: &str = "payments.payment_returned";

/// The type of the accountants' follow-up notice.
pub const FOLLOWUP_TASK: &str = "payments.followup_task";

const MODULE: &str = "payments";
const GROUP: &str = "payments";

/// `date` as `18 Oct 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The template variables of the bank-return notice.
fn returned_data(payment: &Payment) -> renox::serde_json::Value {
    json!({
        "number": payment.number.clone().unwrap_or_default(),
        "amount": money_text(&payment.currency, payment.amount),
        "payment_date": date_text(payment.payment_date),
    })
}

/// The template variables of the follow-up notice.
fn followup_data(payment: &Payment, task: &FollowupTask) -> renox::serde_json::Value {
    json!({
        "text": task.text,
        "number": payment.number.clone().unwrap_or_default(),
        "amount": money_text(&task.currency, task.amount),
        "link": "/payments/tasks",
    })
}

/// The notification type [`PAYMENT_RETURNED`].
pub fn returned_type() -> NotificationTypeDef {
    NotificationTypeDef::new(PAYMENT_RETURNED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.returned.subject")),
            english("wl_payments.returned.body"),
        )
}

/// The notification type [`FOLLOWUP_TASK`].
pub fn followup_type() -> NotificationTypeDef {
    NotificationTypeDef::new(FOLLOWUP_TASK, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.followup.subject")),
            english("wl_payments.followup.body"),
        )
}

/// Registers both types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(returned_type());
    notify::register_type(followup_type());
}

/// Tells the party of `payment` that the bank returned it.
pub(crate) async fn payment_returned(state: &AppState, payment: &Payment) -> Result {
    let request = NotifyRequest::new(PAYMENT_RETURNED, payment.subject())
        .to_party(payment.party())
        .data(returned_data(payment));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells each holder of the accountant role about `task`, one message per
/// user.
pub(crate) async fn followup_created(
    state: &AppState,
    payment: &Payment,
    task: &FollowupTask,
) -> Result {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(());
    }
    let mut request =
        NotifyRequest::new(FOLLOWUP_TASK, payment.subject()).data(followup_data(payment, task));
    for id in ids {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment() -> Payment {
        Payment {
            id: 4,
            party_id: 7,
            number: Some("RCT-PGP-2026-0001".into()),
            amount: 300_000,
            currency: "AED".into(),
            payment_date: NaiveDate::from_ymd_opt(2026, 10, 18).expect("date"),
            ..Default::default()
        }
    }

    #[test]
    fn the_returned_type_is_a_transactional_email_with_english_text() {
        let def = returned_type();
        assert_eq!(def.key, "payments.payment_returned");
        assert_eq!(def.category, Category::Transactional);
        assert_eq!(def.channels, [NotifyChannel::Email]);
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        assert!(template.body.contains("returned by the bank"));
        for variable in ["{{ number }}", "{{ amount }}", "{{ payment_date }}"] {
            assert!(template.body.contains(variable), "{variable}");
        }
    }

    #[test]
    fn the_returned_data_names_the_receipt_the_amount_and_the_date() {
        let def = returned_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        let text = notify::fill(body, &returned_data(&payment()));
        assert!(text.contains("RCT-PGP-2026-0001"));
        assert!(text.contains("AED 3,000.00"));
        assert!(text.contains("18 Oct 2026"));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn the_followup_notice_carries_the_task_text() {
        let task = FollowupTask::raise_charge(&payment(), 50_000, vec![]);
        let data = followup_data(&payment(), &task);
        assert_eq!(data["text"], "Raise a charge for AED 500.00");
        assert_eq!(data["link"], "/payments/tasks");
        let def = followup_type();
        let body = &def
            .template_for(NotifyChannel::Email, "en")
            .expect("en")
            .body;
        assert!(notify::fill(body, &data).contains("Raise a charge for AED 500.00"));
    }
}
