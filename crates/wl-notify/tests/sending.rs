//! Sending, retrying and suppressing messages (wl-notify §4.1 `send`,
//! `fail`, `retry`, `suppress`): the acceptance checks AC3, AC5 and AC6 of
//! #55.

mod support;

use std::time::Duration;

use renox::prelude::*;
use wl_kernel::notify::{NotifyChannel, NotifyRecipient};
use wl_notify::channels::HardError;
use wl_notify::{MessageStatus, ReactionRun, SendError, Suppression, SuppressionReason};

use support::{
    BLUE_DUNES, BLUE_DUNES_EMAIL, QUIET_SANDS, RECEIPT, RED_ROCK, RED_ROCK_EMAIL, REMINDER, boot,
};

/// AC3: a party with no contact point for any channel of the type gets a
/// `suppressed` message and a `MessageSuppressed`, never a give-up.
#[renox::test]
async fn a_party_with_no_contact_point_is_suppressed() {
    let t = boot().await;
    let ids = t.notify_party(REMINDER, QUIET_SANDS).await;
    assert_eq!(ids.len(), 1);
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("no_contact_point"));
    assert_eq!(message.address, None);
    assert_eq!(t.history(ids[0]).await, ["suppress"]);

    let suppressed = t.suppressed.all();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0].message, ids[0]);
    assert_eq!(suppressed[0].reason, "no_contact_point");
    assert_eq!(
        suppressed[0].recipient,
        NotifyRecipient::Party { id: QUIET_SANDS }
    );

    // Nothing to send, nothing gives up, no reaction runs.
    t.app.run_all_jobs().await;
    assert!(t.email.sent().is_empty() && t.sms.sent().is_empty());
    assert!(t.gave_up.all().is_empty());
    assert_eq!(ReactionRun::query().count(t.db()).await.unwrap(), 0);
    assert!(t.calls().is_empty());
}

/// AC3: an address on the suppression list is `suppressed`, not sent.
#[renox::test]
async fn an_address_on_the_list_is_suppressed() {
    let t = boot().await;
    Suppression::create(
        t.db(),
        Suppression {
            address: RED_ROCK_EMAIL.to_owned(),
            channel: NotifyChannel::Email,
            reason: SuppressionReason::OptOut,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ids = t.notify_party(RECEIPT, RED_ROCK).await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("suppressed"));
    assert_eq!(message.address.as_deref(), Some(RED_ROCK_EMAIL));
    t.app.run_all_jobs().await;
    assert!(t.email.sent().is_empty());
    assert_eq!(t.suppressed.all().len(), 1);
    assert!(t.gave_up.all().is_empty());
    assert_eq!(ReactionRun::query().count(t.db()).await.unwrap(), 0);
}

/// AC5: one transient failure, then the e-mail goes out after one `retry`.
#[renox::test]
async fn a_transient_failure_is_retried_and_then_sent() {
    let t = boot().await;
    t.email
        .then([Err(SendError::transient("421", "try again later"))]);
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Queued);
    assert_eq!(message.attempts, 1);
    assert_eq!(message.last_error_code.as_deref(), Some("421"));

    // Not before the minute is up.
    assert_eq!(t.after(Duration::from_secs(50)).await, 0);
    t.after(Duration::from_secs(10)).await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Sent);
    assert_eq!(message.attempts, 2);
    assert_eq!(message.provider.as_deref(), Some("scripted-email"));
    assert_eq!(message.provider_message_id.as_deref(), Some("email-2"));
    assert!(message.sent_at.is_some());
    assert_eq!(t.history(ids[0]).await, ["fail", "retry", "send"]);

    let sent = t.email.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].address, BLUE_DUNES_EMAIL.0);
    assert_eq!(sent[1].title, "Invoice INV-PGP-2026-0007 is overdue");
    assert_eq!(sent[1].body, "Please pay invoice INV-PGP-2026-0007.");
    // No fallback, no give-up.
    assert_eq!(t.messages().await.len(), 1);
    assert!(t.sms.sent().is_empty());
    assert!(t.gave_up.all().is_empty());
    assert_eq!(ReactionRun::query().count(t.db()).await.unwrap(), 0);
}

/// AC6: a hard bounce puts the address on the list, and the next message of
/// any type to it is `suppressed`.
#[renox::test]
async fn a_hard_bounce_suppresses_the_address() {
    let t = boot().await;
    t.email.then([Err(SendError::hard(
        HardError::HardBounce,
        "550",
        "no such user",
    ))]);
    let ids = t.notify_party(RECEIPT, RED_ROCK).await;
    t.run_jobs().await;
    let first = t.message(ids[0]).await;
    assert_eq!(first.status, MessageStatus::Abandoned);
    assert_eq!(first.attempts, 1);
    assert_eq!(first.error_kind.as_deref(), Some("hard_bounce"));

    let listed = Suppression::query().get(t.db()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].address, RED_ROCK_EMAIL);
    assert_eq!(listed[0].channel, NotifyChannel::Email);
    assert_eq!(listed[0].reason, SuppressionReason::HardBounce);
    assert_eq!(listed[0].source, Some(format!("message:{}", first.id)));

    // Another type, to the party and to the bare address: not sent.
    let to_party = t.notify_party(REMINDER, RED_ROCK).await;
    let to_address = t
        .notify(
            RECEIPT,
            NotifyRecipient::Address {
                channel: NotifyChannel::Email,
                address: "AP@RedRock.example".into(),
            },
        )
        .await;
    for id in [to_party[0], to_address[0]] {
        let message = t.message(id).await;
        assert_eq!(message.status, MessageStatus::Suppressed, "{id}");
        assert_eq!(message.suppress_reason.as_deref(), Some("suppressed"));
    }
    t.app.run_all_jobs().await;
    assert_eq!(t.email.sent().len(), 1);
}
