//! The marketing frequency cap in `send` (wl-notify §6 `NTF.RUL.02`; #62
//! part 4): the count of a party's marketing messages sent in 7 days, the
//! suppression and the lock that serialises sends for one party.

mod support;

use std::sync::Arc;

use wl_kernel::consents::Consents;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRecipient};
use wl_notify::{MessageStatus, WlNotify, send};

use support::{
    BLUE_DUNES, EveryoneConsents, QUIET_SANDS, REMINDER, boot_consenting, boot_providers,
};

const SMS_OFFER: &str = "campaign.cap_sms_offer";
const EMAIL_OFFER: &str = "campaign.cap_email_offer";

fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(SMS_OFFER, "campaigns")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms])
            .template(NotifyChannel::Sms, "en", None, "A new offer."),
    );
    notify::register_type(
        NotificationTypeDef::new(EMAIL_OFFER, "campaigns")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some("Offer"), "A new offer."),
    );
}

/// Sends one marketing message of `key` to the party at the clock's time.
async fn offer(t: &support::Booted, key: &str) -> wl_notify::Message {
    let ids = t.notify_party(key, BLUE_DUNES).await;
    t.run_jobs().await;
    t.message(ids[0]).await
}

/// AC3: two sent in 7 days, the third is suppressed `frequency_cap` and the
/// provider is not called; one sent 7 days and 1 minute ago does not count.
#[renox::test]
async fn the_third_marketing_message_in_seven_days_is_suppressed() {
    let t = boot_consenting().await;
    register_types();
    // Sent 7 days and 1 minute before the third.
    t.clock.set("2026-10-12 11:59");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    t.clock.set("2026-10-15 12:00");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);

    // The first is 7 days and 1 minute old: it no longer counts.
    t.clock.set("2026-10-19 12:00");
    let third = offer(&t, SMS_OFFER).await;
    assert_eq!(third.status, MessageStatus::Sent);
    assert_eq!(t.sms.sent().len(), 3);

    // Now two are in the last 7 days (15th and 19th): a fourth is capped.
    let fourth = offer(&t, SMS_OFFER).await;
    assert_eq!(fourth.status, MessageStatus::Suppressed);
    assert_eq!(fourth.suppress_reason.as_deref(), Some("frequency_cap"));
    assert_eq!(fourth.sent_at, None);
    assert_eq!(t.sms.sent().len(), 3);
    assert_eq!(
        t.history(wl_kernel::notify::MessageId(fourth.id)).await,
        ["suppress"]
    );
}

/// AC4: a message suppressed as `frequency_cap` does not count; once a sent
/// one ages out of the 7 days, the next message is sent.
#[renox::test]
async fn a_capped_message_does_not_count_and_old_ones_age_out() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-13 12:00");
    offer(&t, SMS_OFFER).await;
    t.clock.set("2026-10-14 12:00");
    offer(&t, SMS_OFFER).await;
    t.clock.set("2026-10-15 12:00");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Suppressed);
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Suppressed);

    // The 13th is 7 days and 1 minute old: only the 14th counts.
    t.clock.set("2026-10-20 12:01");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    assert_eq!(t.sms.sent().len(), 3);
}

/// AC6: the cap counts across channels.
#[renox::test]
async fn the_cap_counts_across_channels() {
    let t = boot_consenting().await;
    register_types();
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    assert_eq!(offer(&t, EMAIL_OFFER).await.status, MessageStatus::Sent);
    let third = offer(&t, SMS_OFFER).await;
    assert_eq!(third.status, MessageStatus::Suppressed);
    assert_eq!(third.suppress_reason.as_deref(), Some("frequency_cap"));
    assert_eq!(t.sms.sent().len(), 1);
    assert_eq!(t.email.sent().len(), 1);
}

/// The cap is a module setting: with 1, the second message is suppressed.
#[renox::test]
async fn the_cap_is_a_module_setting() {
    let t = boot_providers(
        None,
        |m: WlNotify| m.marketing_frequency_cap(1),
        |app| {
            let consents: Arc<dyn Consents> = Arc::new(EveryoneConsents);
            app.provide(consents)
        },
    )
    .await;
    register_types();
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    let second = offer(&t, SMS_OFFER).await;
    assert_eq!(second.status, MessageStatus::Suppressed);
    assert_eq!(second.suppress_reason.as_deref(), Some("frequency_cap"));
}

/// Other categories, users and bare addresses are never capped or counted.
#[renox::test]
async fn only_marketing_to_a_party_is_capped() {
    let t = boot_consenting().await;
    register_types();
    offer(&t, SMS_OFFER).await;
    offer(&t, EMAIL_OFFER).await;

    // AC9: a reminder to the same party is sent (and not counted).
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);

    // Another party has its own count.
    let ids = t.notify_party(SMS_OFFER, QUIET_SANDS).await;
    t.run_jobs().await;
    assert_ne!(
        t.message(ids[0]).await.suppress_reason.as_deref(),
        Some("frequency_cap")
    );

    // Marketing to a bare address has no party to cap.
    let ids = t
        .notify(
            SMS_OFFER,
            NotifyRecipient::Address {
                channel: NotifyChannel::Sms,
                address: "+971501234000".into(),
            },
        )
        .await;
    t.run_jobs().await;
    assert_ne!(
        t.message(ids[0]).await.suppress_reason.as_deref(),
        Some("frequency_cap")
    );
}

/// `count_recent` counts the sent marketing messages of the party only.
#[renox::test]
async fn count_recent_counts_sent_marketing_of_one_party() {
    let t = boot_consenting().await;
    register_types();
    offer(&t, SMS_OFFER).await;
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);
    let since = wl_notify::cap::since(t.clock.now());
    let state = t.app.state();
    assert_eq!(
        wl_notify::cap::count_recent(state, BLUE_DUNES.0, since)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        wl_notify::cap::count_recent(state, QUIET_SANDS.0, since)
            .await
            .unwrap(),
        0
    );
}

/// AC5: one marketing message in 7 days and two more sent at the same moment:
/// exactly one is sent, the other suppressed.
#[renox::test]
async fn two_concurrent_sends_cannot_both_pass_the_cap() {
    let t = boot_consenting().await;
    register_types();
    offer(&t, SMS_OFFER).await;
    let one = t.notify_party(SMS_OFFER, BLUE_DUNES).await;
    let two = t.notify_party(EMAIL_OFFER, BLUE_DUNES).await;

    let state = t.app.state();
    let (a, b) = renox::tokio::join!(send::send(state, one[0].0), send::send(state, two[0].0));
    a.unwrap();
    b.unwrap();
    t.run_jobs().await;

    let statuses = [
        t.message(one[0]).await.status,
        t.message(two[0]).await.status,
    ];
    let sent = statuses
        .iter()
        .filter(|s| **s == MessageStatus::Sent)
        .count();
    let capped = statuses
        .iter()
        .filter(|s| **s == MessageStatus::Suppressed)
        .count();
    assert_eq!((sent, capped), (1, 1), "{statuses:?}");
}

/// The lock serves one party at a time and never holds another party.
#[renox::test]
async fn the_party_lock_serves_one_send_at_a_time() {
    let first = wl_notify::cap::lock(9_001).await;
    // Another party does not wait.
    drop(wl_notify::cap::lock(9_002).await);
    let waiting = renox::tokio::spawn(async { wl_notify::cap::lock(9_001).await });
    renox::tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    drop(first);
    drop(waiting.await.expect("the waiting send gets the lock"));
}
