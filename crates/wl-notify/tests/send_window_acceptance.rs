//! The consolidated acceptance checks of #62 (`NTF.RUL.02`, part 5): one test
//! for each point of "Done when", each through the real path: the notifier
//! queues, the job sends, the release job lets a held message go, the page
//! `/notify/send-windows` saves the windows, and two broadcasts run through
//! `run_due`.

mod support;

use std::sync::Arc;

use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::broadcast::{
    BroadcastRequest, BroadcastRequesters, RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::Consents;
use wl_kernel::notify::{
    self, Category, ContactKind, NotificationTypeDef, NotifyChannel, PartyContactPoint,
    PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_notify::broadcast_run::run_due;
use wl_notify::permissions as perm;
use wl_notify::{Broadcast, MessageStatus, SendWindow, cap, release, send};

use renox::prelude::*;
use support::{
    BLUE_DUNES, Booted, EveryoneConsents, REMINDER, boot_consenting, boot_providers, dubai,
};

const SMS_OFFER: &str = "campaign.window_sms_offer";
const EMAIL_OFFER: &str = "campaign.window_email_offer";
const PATH: &str = "/notify/send-windows";

fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(SMS_OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms])
            .template(NotifyChannel::Sms, "en", None, "A new offer."),
    );
    notify::register_type(
        NotificationTypeDef::new(EMAIL_OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some("Offer"), "A new offer."),
    );
}

/// One marketing message of `key` to the party, sent by the job at the
/// clock's time.
async fn offer(t: &Booted, key: &str) -> wl_notify::Message {
    let ids = t.notify_party(key, BLUE_DUNES).await;
    t.run_jobs().await;
    t.message(ids[0]).await
}

async fn release_due(t: &Booted) -> usize {
    release::run_due(t.app.state(), t.clock.now())
        .await
        .unwrap()
}

/// A marketing SMS queued at 22:15 is deferred and sent at 07:00 the next
/// day.
#[renox::test]
async fn done_1_a_marketing_sms_at_22_15_is_sent_at_07_00_the_next_day() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-19 22:15");
    let ids = t.notify_party(SMS_OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let held = t.message(ids[0]).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 07:00")));
    assert!(t.sms.sent().is_empty());

    t.clock.set("2026-10-20 07:00");
    assert_eq!(release_due(&t).await, 1);
    t.run_jobs().await;
    let sent = t.message(ids[0]).await;
    assert_eq!(sent.status, MessageStatus::Sent);
    assert_eq!(sent.sent_at, Some(dubai("2026-10-20 07:00")));
    assert_eq!(t.sms.sent().len(), 1);
}

/// With 2 marketing messages sent in the last 7 days, a third is suppressed
/// with reason frequency cap, and `MessageSuppressed` says so.
#[renox::test]
async fn done_2_a_third_marketing_message_in_seven_days_is_suppressed() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-15 12:00");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    t.clock.set("2026-10-17 12:00");
    assert_eq!(offer(&t, EMAIL_OFFER).await.status, MessageStatus::Sent);

    t.clock.set("2026-10-19 12:00");
    let third = offer(&t, SMS_OFFER).await;
    assert_eq!(third.status, MessageStatus::Suppressed);
    assert_eq!(third.suppress_reason.as_deref(), Some("frequency_cap"));
    assert_eq!(third.sent_at, None);
    assert_eq!(t.sms.sent().len(), 1);
    assert_eq!(t.email.sent().len(), 1);
    let events = t.suppressed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "frequency_cap");
}

/// Setting the SMS window to 06:00-22:00 is refused; 09:00-20:00 is accepted
/// and the next send obeys it.
#[renox::test]
async fn done_3_the_sms_window_cannot_be_widened_and_can_be_narrowed() {
    let t = boot_consenting().await;
    register_types();
    let admin = t.user("Amal", &[perm::CHANNELS_MANAGE]).await;
    t.app.acting_as(&admin);

    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "sms"), ("start", "06:00"), ("end", "22:00")],
        )
        .await
        .assert_status(422)
        .assert_see("SMS marketing may only be sent 07:00-21:00");
    t.clock.set("2026-10-19 06:30");
    let early = t.notify_party(SMS_OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(early[0]).await.status, MessageStatus::Deferred);

    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "sms"), ("start", "09:00"), ("end", "20:00")],
        )
        .await
        .assert_status(303);
    t.clock.set("2026-10-19 20:30");
    let late = t.notify_party(SMS_OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let held = t.message(late[0]).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 09:00")));
    t.clock.set("2026-10-19 10:00");
    let inside = t.notify_party(SMS_OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(inside[0]).await.status, MessageStatus::Sent);
}

/// Every party 1.. is active and has one mobile point.
struct Everyone;

impl PartyDirectory for Everyone {
    fn find<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn find_by_code<'a>(
        &'a self,
        _db: &'a Db,
        _code: &'a str,
    ) -> PartyFuture<'a, Option<PartySummary>> {
        Box::pin(async { Ok(None) })
    }

    fn display_name<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _lang: &'a str,
    ) -> PartyFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn status<'a>(&'a self, _db: &'a Db, _party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        Box::pin(async { Ok(Some(PartyStatus::Active)) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: "en".into(),
                preferred_channel: None,
                points: vec![PartyContactPoint {
                    id: party.0,
                    kind: ContactKind::Mobile,
                    address: format!("+9715{:08}", party.0),
                    primary: true,
                    verified: true,
                    valid: true,
                }],
            }))
        })
    }
}

/// Every segment holds party 1 only.
struct JustOne;

impl Segments for JustOne {
    fn members<'a>(
        &'a self,
        _db: &'a Db,
        _segment: SegmentId,
        _at: renox::db::DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        Box::pin(async { Ok(vec![PartyId(1)]) })
    }
}

/// Every campaign is still active.
struct Active;

impl BroadcastRequesters for Active {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
        _at: renox::db::DateTime,
    ) -> RequesterFuture<'a, RequesterCheck> {
        Box::pin(async { Ok(RequesterCheck::Active) })
    }
}

/// Asks for a broadcast of the SMS offer to segment `n` of campaign `n`.
async fn ask(t: &Booted, n: i64, by: i64) -> Broadcast {
    let send_at = t.clock.now() + renox::chrono::Duration::hours(2);
    let request = BroadcastRequest::new(
        SMS_OFFER,
        Subject::new("sales.campaign", n),
        by,
        NotifyChannel::Sms,
        SegmentId(n),
        send_at,
    );
    let id = notify::notifier(t.app.state())
        .broadcast(t.app.state(), request)
        .await
        .unwrap();
    Broadcast::find_or_404(t.db(), id.0).await.unwrap()
}

/// Recipient R has 1 marketing message in the last 7 days and two broadcasts
/// release a message to R at the same moment: one is sent, the other is
/// suppressed frequency cap.
#[renox::test]
async fn done_4_two_broadcasts_releasing_at_once_cannot_both_pass_the_cap() {
    register_types();
    let t = boot_providers(
        Some(Arc::new(Everyone)),
        |m| m.sms_sender_id("AD-SCHOOL"),
        |app| {
            let segments: Arc<dyn Segments> = Arc::new(JustOne);
            let consents: Arc<dyn Consents> = Arc::new(EveryoneConsents);
            let requesters: Arc<dyn BroadcastRequesters> = Arc::new(Active);
            app.provide(segments).provide(consents).provide(requesters)
        },
    )
    .await;
    let marketer = t.user("Aisha", &[perm::MARKETING_SEND]).await;
    t.clock.set("2026-10-17 12:00");
    let earlier = t.notify_party(SMS_OFFER, PartyId(1)).await;
    t.run_jobs().await;
    assert_eq!(t.message(earlier[0]).await.status, MessageStatus::Sent);

    t.clock.set("2026-10-19 12:00");
    let first = ask(&t, 1, marketer.id).await;
    let second = ask(&t, 2, marketer.id).await;
    assert_eq!(run_due(t.app.state(), first.send_at).await.unwrap(), 2);
    let queued: Vec<_> = t
        .messages()
        .await
        .into_iter()
        .filter(|m| m.broadcast_id.is_some())
        .collect();
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().any(|m| m.broadcast_id == Some(first.id)));
    assert!(queued.iter().any(|m| m.broadcast_id == Some(second.id)));

    // Two tasks of their own, each sending one message: the per-party lock
    // is what lets only one of them pass the cap.
    let state = t.app.state();
    let tasks: Vec<_> = queued
        .iter()
        .map(|message| {
            let (state, id) = (state.clone(), message.id);
            renox::tokio::spawn(async move { send::send(&state, id).await })
        })
        .collect();
    for task in tasks {
        task.await.expect("the task ran").unwrap();
    }
    t.run_jobs().await;

    let mut statuses = Vec::new();
    let mut reasons = Vec::new();
    for message in &queued {
        let stored = t.message(notify::MessageId(message.id)).await;
        statuses.push(stored.status);
        reasons.push(stored.suppress_reason);
    }
    let sent = statuses
        .iter()
        .filter(|s| **s == MessageStatus::Sent)
        .count();
    let capped = statuses
        .iter()
        .filter(|s| **s == MessageStatus::Suppressed)
        .count();
    assert_eq!((sent, capped), (1, 1), "{statuses:?}");
    assert!(
        reasons
            .iter()
            .flatten()
            .all(|reason| reason == "frequency_cap")
    );
    // The earlier message and the one that passed: the cap of 2 is reached,
    // not passed.
    let since = cap::since(t.clock.now());
    assert_eq!(cap::count_recent(state, 1, since).await.unwrap(), 2);
    assert_eq!(t.sms.sent().len(), 2);
}

/// Setting the e-mail window to 06:00-23:00 is accepted, and an e-mail offer
/// at 22:30 is then sent at once; at 23:00 it waits for 06:00.
#[renox::test]
async fn done_5_the_email_window_can_be_widened_to_06_00_23_00() {
    let t = boot_consenting().await;
    register_types();
    let admin = t.user("Amal", &[perm::CHANNELS_MANAGE]).await;
    t.app.acting_as(&admin);
    t.app
        .htmx()
        .post(
            PATH,
            &[("channel", "email"), ("start", "06:00"), ("end", "23:00")],
        )
        .await
        .assert_status(303);
    let stored = SendWindow::for_channel(t.db(), NotifyChannel::Email)
        .await
        .unwrap();
    assert_eq!((stored.start, stored.end), (360, 1380));

    t.clock.set("2026-10-19 22:30");
    assert_eq!(offer(&t, EMAIL_OFFER).await.status, MessageStatus::Sent);
    t.clock.set("2026-10-19 23:00");
    let ids = t.notify_party(EMAIL_OFFER, BLUE_DUNES).await;
    t.run_jobs().await;
    let held = t.message(ids[0]).await;
    // Blue Dunes has already had one marketing message today, so a second
    // would still pass the cap of 2; it waits only for the window.
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 06:00")));
}

/// A normal booking reminder to a customer queued at 22:15 is deferred by
/// quiet hours and sent at 08:00 (not 07:00); even when the customer has
/// reached the marketing cap it is never suppressed frequency cap, and it
/// does not count towards the cap.
#[renox::test]
async fn done_6_a_reminder_is_held_by_quiet_hours_and_never_capped() {
    let t = boot_consenting().await;
    register_types();
    t.clock.set("2026-10-18 12:00");
    assert_eq!(offer(&t, SMS_OFFER).await.status, MessageStatus::Sent);
    assert_eq!(offer(&t, EMAIL_OFFER).await.status, MessageStatus::Sent);
    let since = cap::since(dubai("2026-10-19 22:15"));
    assert_eq!(
        cap::count_recent(t.app.state(), BLUE_DUNES.0, since)
            .await
            .unwrap(),
        2
    );

    t.clock.set("2026-10-19 22:15");
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    let held = t.message(ids[0]).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 08:00")));

    t.clock.set("2026-10-20 07:00");
    assert_eq!(release_due(&t).await, 0);
    t.clock.set("2026-10-20 08:00");
    assert_eq!(release_due(&t).await, 1);
    t.run_jobs().await;
    let sent = t.message(ids[0]).await;
    assert_eq!(sent.status, MessageStatus::Sent);
    assert_eq!(sent.suppress_reason, None);
    assert_eq!(sent.sent_at, Some(dubai("2026-10-20 08:00")));

    // It did not count: the count of the 7 days up to 08:00 is still the two
    // marketing messages.
    let since = cap::since(t.clock.now());
    assert_eq!(
        cap::count_recent(t.app.state(), BLUE_DUNES.0, since)
            .await
            .unwrap(),
        2
    );
}
