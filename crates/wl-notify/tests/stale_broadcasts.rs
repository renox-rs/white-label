//! Stale broadcasts and stopping a broadcast (#63, `NTF.RUL.03`): the
//! unapproved broadcast cancelled a day after its send time, the marketing
//! messages dropped 48 hours after the start, and the stop from the page.
//! Every check goes through the module's own run, the send job or the route.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use renox::chrono::Duration;
use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastCancelReason, BroadcastCancelled, BroadcastId, BroadcastRequest, BroadcastRequesters,
    BroadcastStopped, RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, FALLBACK_KIND, NotificationTypeDef, NotifyChannel,
    PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{PartyId, Subject, approvals};
use wl_notify::broadcast_results;
use wl_notify::broadcast_run::{Batching, Ran, run_due, run_one};
use wl_notify::permissions::{LOG_VIEW, MARKETING_SEND};
use wl_notify::stale;
use wl_notify::{Broadcast, BroadcastStatus, MessageStatus};

use support::{Booted, Recorded, boot_providers, record};

const OFFER: &str = "campaign.offer";

/// Every party 1..=1000 is active and has one mobile point.
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

/// Segment members by segment id.
struct Lists(BTreeMap<i64, Vec<PartyId>>);

impl Segments for Lists {
    fn members<'a>(
        &'a self,
        _db: &'a Db,
        segment: SegmentId,
        _at: renox::db::DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        let found = self.0.get(&segment.0).cloned().unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }
}

/// Marketing consent for the parties in the set.
struct Granted(BTreeSet<i64>);

impl Consents for Granted {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let yes = purpose == "marketing" && self.0.contains(&party.0);
        Box::pin(async move { Ok(yes) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
        _source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}


/// The campaign is always active.
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

fn ids(range: std::ops::RangeInclusive<i64>) -> Vec<PartyId> {
    range.map(PartyId).collect()
}

/// The app, with the segments below: segment 1 has 150 members of which 100
/// consent, segment 3 has 350 who all consent (above the approval limit).
async fn boot() -> (
    Booted,
    Recorded<BroadcastCancelled>,
    Recorded<BroadcastStopped>,
) {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms])
            .template(NotifyChannel::Sms, "en", None, "Offer for you"),
    );
    let cancelled = Recorded::new();
    let stopped = Recorded::new();
    let (c, s) = (cancelled.clone(), stopped.clone());
    let t = boot_providers(
        Some(Arc::new(Everyone)),
        |m| m.sms_sender_id("AD-SCHOOL"),
        move |app| {
            let lists = Lists(BTreeMap::from([
                (1, [ids(1..=100), ids(351..=400)].concat()),
                (3, ids(1..=350)),
            ]));
            let segments: Arc<dyn Segments> = Arc::new(lists);
            let consents: Arc<dyn Consents> = Arc::new(Granted((1..=350).collect()));
            let requesters: Arc<dyn BroadcastRequesters> = Arc::new(Active);
            let app = app.provide(segments).provide(consents).provide(requesters);
            record(record(app, &c), &s)
        },
    )
    .await;
    (t, cancelled, stopped)
}

async fn stored(t: &Booted, id: BroadcastId) -> Broadcast {
    Broadcast::find_or_404(t.db(), id.0).await.unwrap()
}

/// Asks for a broadcast of `segment` for `by`, going out two hours from the
/// test clock.
async fn ask(t: &Booted, segment: i64, by: i64) -> BroadcastId {
    let send_at = t.clock.now() + Duration::hours(2);
    let request = BroadcastRequest::new(
        OFFER,
        Subject::new("sales.campaign", 7),
        by,
        NotifyChannel::Sms,
        SegmentId(segment),
        send_at,
    );
    notify::notifier(t.app.state())
        .broadcast(t.app.state(), request)
        .await
        .unwrap()
}

async fn statuses(t: &Booted) -> Vec<MessageStatus> {
    t.messages().await.iter().map(|m| m.status).collect()
}

fn count(all: &[MessageStatus], status: MessageStatus) -> usize {
    all.iter().filter(|s| **s == status).count()
}

/// Done when 1: a broadcast waiting for approval at its send time plus a day
/// is cancelled with *approval expired* at the first check, the requester is
/// told and `BroadcastCancelled` is emitted once.
#[renox::test]
async fn an_unapproved_broadcast_is_cancelled_a_day_after_its_send_time() {
    let (t, cancelled, _) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::PendingApproval);
    let due = b.send_at + Duration::hours(24);

    // A minute short of the limit nothing happens.
    let early = stale::run(t.app.state(), due - Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(early, stale::Swept::default());
    assert_eq!(
        stored(&t, id).await.status,
        BroadcastStatus::PendingApproval
    );

    // The check at the limit (the body of the 15-minute task).
    let swept = stale::run(t.app.state(), due).await.unwrap();
    assert_eq!(swept.expired, 1);
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("approval_expired"));
    let events = cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].broadcast, id);
    assert_eq!(events[0].reason, BroadcastCancelReason::ApprovalExpired);
    let notices = t.notices(&a, FALLBACK_KIND).await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("approval_expired"), "{}", notices[0]);

    // The next check changes nothing and announces nothing.
    let again = stale::run(t.app.state(), due + Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(again.expired, 0);
    assert_eq!(cancelled.all().len(), 1);
}

/// A broadcast approved before the check runs is not expired.
#[renox::test]
async fn a_broadcast_approved_in_time_is_not_expired() {
    let (t, cancelled, _) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let h = t.user("Hana", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    let b = stored(&t, id).await;
    let request = b.approval_request_id.unwrap();
    approvals::approve(t.app.state(), request, &h, "")
        .await
        .unwrap();

    let swept = stale::run(t.app.state(), b.send_at + Duration::hours(25))
        .await
        .unwrap();
    assert_eq!(swept.expired, 0);
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Scheduled);
    assert!(cancelled.all().is_empty());
}

/// Done when 2: queued messages that no provider took are suppressed
/// *expired* 48 hours after the start, counted as excluded, and none is sent
/// when the provider is back.
#[renox::test]
async fn messages_still_queued_48_hours_after_the_start_are_suppressed_and_never_sent() {
    let (t, _, _) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    let b = stored(&t, id).await;
    assert_eq!(b.started_at, Some(send_at), "start ran on the module clock");
    assert_eq!(count(&statuses(&t).await, MessageStatus::Queued), 100);

    let early = send_at + Duration::hours(48) - Duration::minutes(1);
    assert_eq!(stale::suppress_stale(t.app.state(), early).await.unwrap(), 0);
    let at_limit = send_at + Duration::hours(48);
    assert_eq!(
        stale::suppress_stale(t.app.state(), at_limit).await.unwrap(),
        100
    );
    let all = t.messages().await;
    assert!(all.iter().all(|m| m.status == MessageStatus::Suppressed));
    assert!(
        all.iter()
            .all(|m| m.suppress_reason.as_deref() == Some("expired"))
    );
    assert_eq!(t.suppressed.all().len(), 100);
    let counts = broadcast_results::excluded(t.db(), id.0, &b.excluded.0)
        .await
        .unwrap();
    assert_eq!(counts.of("expired"), 100);

    // The provider is back: the queued send jobs find nothing to send.
    t.run_jobs().await;
    assert!(t.sms.sent().is_empty());
    // A second sweep finds nothing.
    let later = send_at + Duration::hours(60);
    assert_eq!(stale::suppress_stale(t.app.state(), later).await.unwrap(), 0);
}

/// A message whose send job only runs after the limit is refused at the send,
/// before the sweep has seen it.
#[renox::test]
async fn a_send_job_that_runs_after_the_stale_limit_sends_nothing() {
    let (t, _, _) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    run_due(t.app.state(), send_at).await.unwrap();

    t.clock.set("2026-10-21 15:00");
    t.run_jobs().await;
    assert!(t.sms.sent().is_empty());
    let all = t.messages().await;
    assert_eq!(all.len(), 100);
    assert!(all.iter().all(
        |m| m.status == MessageStatus::Suppressed
            && m.suppress_reason.as_deref() == Some("expired")
    ));
}

/// The stale limit counts from the start, not from a deferral: a message
/// released after a long deferral inside the limit is sent.
#[renox::test]
async fn a_message_deferred_overnight_is_not_stale() {
    let (t, _, _) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    t.clock.set("2026-10-19 22:15");
    run_due(t.app.state(), send_at).await.unwrap();
    t.run_jobs().await;
    assert_eq!(count(&statuses(&t).await, MessageStatus::Deferred), 100);

    t.clock.set("2026-10-20 07:00");
    let swept = stale::suppress_stale(t.app.state(), t.clock.now())
        .await
        .unwrap();
    assert_eq!(swept, 0);
    wl_notify::release::run_due(t.app.state(), t.clock.now())
        .await
        .unwrap();
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 100);
}

/// Done when 3: stopping a `sending` broadcast suppresses its queued messages
/// with *broadcast stopped*, emits `BroadcastStopped`, leaves the sent ones
/// sent and queues no more.
#[renox::test]
async fn stopping_a_sending_broadcast_suppresses_what_is_not_sent() {
    let (t, _, stopped) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let h = t.user("Hana", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;

    // 30 messages queued and sent, 35 more queued, 35 not queued yet.
    let first = Batching {
        size: 30,
        max_batches: Some(1),
    };
    assert_eq!(
        run_one(t.app.state(), id.0, send_at, first).await.unwrap(),
        Ran::Partial
    );
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 30);
    let second = Batching {
        size: 35,
        max_batches: Some(1),
    };
    assert_eq!(
        run_one(t.app.state(), id.0, send_at, second).await.unwrap(),
        Ran::Partial
    );
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Sending);

    t.app.acting_as(&h);
    t.app
        .htmx()
        .post(&format!("/notify/broadcasts/{}/stop", id.0), &[])
        .await
        .assert_status(303);

    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Sending, "the state stays");
    assert_eq!(b.stopped_by, Some(h.id));
    assert!(b.stopped_at.is_some());
    let all = statuses(&t).await;
    assert_eq!(count(&all, MessageStatus::Sent), 30);
    assert_eq!(count(&all, MessageStatus::Suppressed), 35);
    let events = stopped.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].broadcast, id);
    assert_eq!(events[0].stopped_by, h.id);
    assert_eq!(events[0].suppressed, 35);
    assert_eq!(t.notices(&a, FALLBACK_KIND).await.len(), 1);
    let counts = broadcast_results::excluded(t.db(), id.0, &b.excluded.0)
        .await
        .unwrap();
    assert_eq!(counts.of("broadcast_stopped"), 35);

    // The rest of the run queues nothing more and completes.
    run_due(t.app.state(), send_at).await.unwrap();
    // The stop notice to the requester is a message of its own, not the
    // broadcast's.
    let queued = t
        .messages()
        .await
        .iter()
        .filter(|m| m.broadcast_id == Some(id.0))
        .count();
    assert_eq!(queued, 65);
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Completed);
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 30);
}

/// Done when 4: stopping a `completed` broadcast with messages deferred until
/// 07:00 suppresses them; the sent ones stay sent.
#[renox::test]
async fn stopping_a_completed_broadcast_drops_its_deferred_messages() {
    let (t, _, stopped) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let h = t.user("Hana", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;

    let first = Batching {
        size: 85,
        max_batches: Some(1),
    };
    run_one(t.app.state(), id.0, send_at, first).await.unwrap();
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 85);

    t.clock.set("2026-10-19 22:15");
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    t.run_jobs().await;
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Completed);
    assert_eq!(count(&statuses(&t).await, MessageStatus::Deferred), 15);

    t.app.acting_as(&h);
    t.app
        .htmx()
        .post(&format!("/notify/broadcasts/{}/stop", id.0), &[])
        .await
        .assert_status(303);
    let all = statuses(&t).await;
    assert_eq!(count(&all, MessageStatus::Suppressed), 15);
    assert_eq!(count(&all, MessageStatus::Sent), 85);
    assert_eq!(stopped.all().len(), 1);
    assert_eq!(stopped.all()[0].suppressed, 15);

    // Released at 07:00 nothing is left to send.
    t.clock.set("2026-10-20 07:00");
    wl_notify::release::run_due(t.app.state(), t.clock.now())
        .await
        .unwrap();
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 85);

    // A second stop has nothing left to stop.
    t.app
        .htmx()
        .post(&format!("/notify/broadcasts/{}/stop", id.0), &[])
        .await
        .assert_status(422);
    assert_eq!(stopped.all().len(), 1);
}

/// Done when 5: only a holder of `notifications.marketing.send` stops a
/// broadcast, the requester without it included; the page shows the button
/// to a holder only.
#[renox::test]
async fn a_user_without_the_permission_cannot_stop_a_broadcast() {
    let (t, _, stopped) = boot().await;
    let rita = t.user("Rita", &[LOG_VIEW]).await;
    let nobody = t.user("Nadia", &[LOG_VIEW]).await;
    let h = t.user("Hana", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, rita.id).await;
    let send_at = stored(&t, id).await.send_at;
    run_due(t.app.state(), send_at).await.unwrap();
    let path = format!("/notify/broadcasts/{}", id.0);

    for user in [&rita, &nobody] {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(&format!("{path}/stop"), &[])
            .await
            .assert_status(403);
    }
    assert!(stopped.all().is_empty());
    assert_eq!(count(&statuses(&t).await, MessageStatus::Queued), 100);

    t.app.acting_as(&rita);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_dont_see("Stop sending");
    t.app.acting_as(&h);
    t.app.get(&path).await.assert_ok().assert_see("Stop sending");
}

/// Stopping a broadcast that has not started, or one with everything sent, is
/// refused.
#[renox::test]
async fn a_scheduled_or_fully_sent_broadcast_has_nothing_to_stop() {
    let (t, _, stopped) = boot().await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let h = t.user("Hana", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let path = format!("/notify/broadcasts/{}/stop", id.0);
    t.app.acting_as(&h);
    t.app.htmx().post(&path, &[]).await.assert_status(422);

    let send_at = stored(&t, id).await.send_at;
    run_due(t.app.state(), send_at).await.unwrap();
    t.run_jobs().await;
    assert_eq!(t.sms.sent().len(), 100);
    t.app.htmx().post(&path, &[]).await.assert_status(422);
    assert!(stopped.all().is_empty());
}
