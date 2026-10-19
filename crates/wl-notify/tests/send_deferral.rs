//! Deferring and releasing (wl-notify §4.1 `defer`, `release`; #62 part 3):
//! the send window of marketing, the quiet hours of normal messages to
//! outsiders, and the release of what was held.

mod support;

use wl_kernel::notify::{
    self, Category, NotificationTypeDef, NotifyChannel, NotifyRecipient, Urgency,
};
use wl_notify::window::Window;
use wl_notify::{MessageStatus, SendWindow, release, send};

use support::{BLUE_DUNES, Booted, REMINDER, boot, boot_consenting, dubai};

const OFFER: &str = "campaign.deferral_offer";
const URGENT_REMINDER: &str = "invoice.deferral_urgent";

fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "campaigns")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
            .template(NotifyChannel::Sms, "en", None, "A new offer.")
            .template(NotifyChannel::Email, "en", Some("Offer"), "A new offer."),
    );
    notify::register_type(
        NotificationTypeDef::new(URGENT_REMINDER, "invoicing")
            .category(Category::Reminder)
            .urgency(Urgency::Urgent)
            .channels(&[NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some("Urgent"), "Pay now."),
    );
}

async fn release_due(t: &Booted) -> usize {
    release::run_due(t.app.state(), t.clock.now())
        .await
        .unwrap()
}

/// AC1: a marketing SMS queued at 22:15 waits until 07:00 the next day, the
/// provider is not called, and the release sends it.
#[renox::test]
async fn a_marketing_sms_at_night_is_deferred_and_released_in_the_morning() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-19 22:15");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;

    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Deferred);
    assert_eq!(message.channel, NotifyChannel::Sms);
    assert_eq!(message.scheduled_for, Some(dubai("2026-10-20 07:00")));
    assert!(t.sms.sent().is_empty());
    assert_eq!(t.history(ids[0]).await, ["defer"]);

    t.clock.set("2026-10-20 06:59");
    assert_eq!(release_due(&t).await, 0);

    t.clock.set("2026-10-20 07:00");
    assert_eq!(release_due(&t).await, 1);
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Queued);
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);
    assert_eq!(t.sms.sent().len(), 1);
    assert_eq!(t.history(ids[0]).await, ["defer", "release", "send"]);
}

/// AC2: inside the window it is sent at once; at 06:59 it waits for 07:00 the
/// same day; at 21:00 sharp it waits for the next morning.
#[renox::test]
async fn the_window_edges_are_07_00_and_21_00() {
    let t = boot_consenting().await;
    register_types();
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);

    t.clock.set("2026-10-19 06:59");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let early = t.message(ids[0]).await;
    assert_eq!(early.status, MessageStatus::Deferred);
    assert_eq!(early.scheduled_for, Some(dubai("2026-10-19 07:00")));

    t.clock.set("2026-10-19 21:00");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let late = t.message(ids[0]).await;
    assert_eq!(late.status, MessageStatus::Deferred);
    assert_eq!(late.scheduled_for, Some(dubai("2026-10-20 07:00")));
}

/// AC7 (the effect): a narrowed stored window defers at 20:30.
#[renox::test]
async fn the_stored_window_of_the_channel_decides() {
    let t = boot_consenting().await;
    register_types();
    let window = Window {
        start: 9 * 60,
        end: 20 * 60,
    };
    SendWindow::save_checked(t.db(), NotifyChannel::Sms, window)
        .await
        .unwrap()
        .unwrap();
    t.clock.set("2026-10-19 20:30");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Deferred);
    assert_eq!(message.scheduled_for, Some(dubai("2026-10-20 09:00")));
}

/// AC8: a normal reminder to a customer at 22:15 waits for 08:00 (not
/// 07:00); an urgent one and a message to a user are sent at once.
#[renox::test]
async fn quiet_hours_hold_a_normal_reminder_to_outsiders_only() {
    let t = boot().await;
    register_types();
    t.clock.set("2026-10-19 22:15");

    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    let urgent = t.notify_party(URGENT_REMINDER, BLUE_DUNES).await;
    let staff = t.user("Sam", &[]).await;
    let to_user = t
        .notify(REMINDER, NotifyRecipient::User { id: staff.id })
        .await;
    t.run_jobs().await;

    let held = t.message(ids[0]).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 08:00")));
    assert_eq!(t.message(urgent[0]).await.status, MessageStatus::Sent);
    assert_eq!(t.message(to_user[0]).await.status, MessageStatus::Sent);

    t.clock.set("2026-10-20 07:30");
    assert_eq!(release_due(&t).await, 0);
    t.clock.set("2026-10-20 08:00");
    assert_eq!(release_due(&t).await, 1);
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);
}

/// AC10: running the release twice sends once; a message released after its
/// window was narrowed is deferred again.
#[renox::test]
async fn a_release_is_once_and_the_send_checks_the_window_again() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-19 22:15");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;

    t.clock.set("2026-10-20 07:00");
    assert_eq!(release_due(&t).await, 1);
    assert_eq!(release_due(&t).await, 0);
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 1);
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);

    // A second message, released after the window moved to 09:00-20:00.
    t.clock.set("2026-10-20 22:00");
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let window = Window {
        start: 9 * 60,
        end: 20 * 60,
    };
    SendWindow::save_checked(t.db(), NotifyChannel::Sms, window)
        .await
        .unwrap()
        .unwrap();
    t.clock.set("2026-10-21 07:00");
    assert_eq!(release_due(&t).await, 1);
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Deferred);
    assert_eq!(message.scheduled_for, Some(dubai("2026-10-21 09:00")));
    assert_eq!(t.sms.sent().len(), 1);
}

/// `send_at` takes "now" as given and answers the time a message waits for.
#[renox::test]
async fn send_at_answers_with_the_time_a_message_waits_for() {
    let t = boot_consenting().await;
    register_types();
    let ids = t.notify_party(OFFER, BLUE_DUNES).await;
    // The job is not run: call send_at directly, at night.
    let night = dubai("2026-10-19 23:00");
    let outcome = send::send_at(t.app.state(), ids[0].0, night).await.unwrap();
    assert_eq!(
        outcome,
        send::SendOutcome::Deferred(dubai("2026-10-20 07:00"))
    );
    // No longer queued: a second call does nothing.
    let again = send::send_at(t.app.state(), ids[0].0, night).await.unwrap();
    assert_eq!(again, send::SendOutcome::Skipped);
}
