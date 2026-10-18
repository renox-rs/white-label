//! The notices the module sends through the kernel's `Notifier`
//! (wl-payments A9, A10).
//!
//! - `payments.payment_returned`: the party is told its payment was
//!   returned by the bank (transactional, e-mail, EN). Nothing is sent for
//!   `entered_in_error`.
//! - `payments.followup_task`: the holders of the accountant role get a
//!   notice for a new follow-up task.
//!
//! - `payments.refund_failed`: the requester and the accountants are told a
//!   refund failed (transactional, bell and e-mail).
//! - `payments.refund_delayed`: the party is told its refund is delayed
//!   (transactional, e-mail, EN); skipped while the party is blocked.
//!
//! While `notify` is off the kernel's `NoNotifier` sends nothing to a party
//! and a bell notice to each user.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::parties::PartyStatus;
use wl_kernel::payments::RefundFailureKind;

use crate::followup::{ACCOUNTANT_ROLE, FollowupTask, money_text};
use crate::payment::Payment;
use crate::refund::Refund;
use crate::texts::english;

/// The type of the party's bank-return notice.
pub const PAYMENT_RETURNED: &str = "payments.payment_returned";

/// The type of the accountants' follow-up notice.
pub const FOLLOWUP_TASK: &str = "payments.followup_task";

/// The type of the staff's notice that a refund failed.
pub const REFUND_FAILED: &str = "payments.refund_failed";

/// The type of the party's notice that its refund is delayed.
pub const REFUND_DELAYED: &str = "payments.refund_delayed";

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

/// The notification type [`REFUND_FAILED`].
pub fn refund_failed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_FAILED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_failed.subject")),
            english("wl_payments.refund_failed.body"),
        )
}

/// The notification type [`REFUND_DELAYED`].
pub fn refund_delayed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REFUND_DELAYED, MODULE)
        .group(GROUP)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_payments.refund_delayed.subject")),
            english("wl_payments.refund_delayed.body"),
        )
}

/// Registers the types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(returned_type());
    notify::register_type(followup_type());
    notify::register_type(refund_failed_type());
    notify::register_type(refund_delayed_type());
}

/// The staff-facing text of a failure kind.
fn kind_text(kind: RefundFailureKind) -> &'static str {
    match kind {
        RefundFailureKind::GatewayDeclined => "Declined by the gateway",
        RefundFailureKind::CardUnavailable => "Card unavailable",
        RefundFailureKind::BankReturn => "Returned by the bank",
    }
}

/// The template variables of the staff's refund-failed notice.
fn refund_failed_data(
    refund: &Refund,
    party_name: &str,
    kind: RefundFailureKind,
    reason: &str,
) -> renox::serde_json::Value {
    json!({
        "refund": format!("#{}", refund.id),
        "party": party_name,
        "amount": money_text(&refund.currency, refund.amount),
        "kind": kind_text(kind),
        "reason": reason,
        "link": format!("/refunds/{}", refund.id),
    })
}

/// The template variables of the party's refund-delayed notice.
fn refund_delayed_data(refund: &Refund) -> renox::serde_json::Value {
    json!({ "amount": money_text(&refund.currency, refund.amount) })
}

/// The users told that a refund failed: the one who requested it and the
/// holders of the accountant role, each once.
pub(crate) fn staff_ids(requested_by: Option<i64>, accountants: &[i64]) -> Vec<i64> {
    let mut ids: Vec<i64> = requested_by
        .into_iter()
        .chain(accountants.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Whether the party hears that its refund is delayed: not while it is
/// blocked (wl-parties suppresses automatic messages to a blocked party).
pub(crate) fn tells_party(status: Option<PartyStatus>) -> bool {
    status != Some(PartyStatus::Blocked)
}

/// After a committed `fail` of `refund`: the requester and the accountants
/// get [`REFUND_FAILED`], and the party gets [`REFUND_DELAYED`] unless it is
/// blocked. Once per failure.
pub(crate) async fn refund_failed(
    state: &AppState,
    refund: &Refund,
    kind: RefundFailureKind,
    reason: &str,
) -> Result {
    let directory = wl_kernel::parties::directory(state);
    let party = refund.party();
    let name = directory
        .find(&state.db, party)
        .await?
        .map_or_else(|| party.to_string(), |found| found.display_name);
    let accountants: Vec<i64> = users_with_role_in(&state.db, ACCOUNTANT_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    let ids = staff_ids(refund.requested_by, &accountants);
    if !ids.is_empty() {
        let mut request = NotifyRequest::new(REFUND_FAILED, refund.refund_id().subject())
            .data(refund_failed_data(refund, &name, kind, reason));
        for id in ids {
            request = request.to_user(id);
        }
        notify::notifier(state).notify(state, request).await?;
    }
    let status = directory.status(&state.db, party).await?;
    if tells_party(status) {
        let request = NotifyRequest::new(REFUND_DELAYED, refund.refund_id().subject())
            .to_party(party)
            .data(refund_delayed_data(refund));
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
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

    fn refund() -> Refund {
        Refund {
            id: 5,
            party_id: 7,
            amount: 100_000,
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_refund_types_are_transactional_emails() {
        for (def, key) in [
            (refund_failed_type(), "payments.refund_failed"),
            (refund_delayed_type(), "payments.refund_delayed"),
        ] {
            assert_eq!(def.key, key);
            assert_eq!(def.category, Category::Transactional);
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        }
    }

    #[test]
    fn the_failed_notice_names_the_refund_the_amount_the_kind_and_the_reason() {
        let def = refund_failed_type();
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        let data = refund_failed_data(
            &refund(),
            "Acme LLC",
            RefundFailureKind::BankReturn,
            "account closed",
        );
        let text = notify::fill(&template.body, &data);
        for part in [
            "#5",
            "Acme LLC",
            "AED 1,000.00",
            "Returned by the bank",
            "account closed",
            "/refunds/5",
        ] {
            assert!(text.contains(part), "{part}");
        }
        assert!(!text.contains("{{"));
    }

    #[test]
    fn the_delayed_notice_tells_the_party_the_amount() {
        let def = refund_delayed_type();
        let template = def.template_for(NotifyChannel::Email, "en").expect("en");
        let text = notify::fill(&template.body, &refund_delayed_data(&refund()));
        assert!(text.contains("your refund of AED 1,000.00 is delayed"));
        assert!(text.contains("we will contact you"));
    }

    #[test]
    fn staff_are_told_once_each() {
        assert_eq!(staff_ids(Some(3), &[5, 3, 4, 5]), [3, 4, 5]);
        assert_eq!(staff_ids(None, &[]), Vec::<i64>::new());
        assert_eq!(staff_ids(None, &[2]), [2]);
    }

    #[test]
    fn a_blocked_party_is_not_told() {
        assert!(!tells_party(Some(PartyStatus::Blocked)));
        assert!(tells_party(Some(PartyStatus::Active)));
        assert!(tells_party(None));
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
