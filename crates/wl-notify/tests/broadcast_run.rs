//! Starting and completing broadcasts (#61 part 5): the due run, the requester
//! re-check, queueing in batches that a retry finishes, the counts and the
//! completion announced once.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastCancelReason, BroadcastCancelled, BroadcastCompleted, BroadcastId, BroadcastRequest,
    BroadcastRequesters, RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, FALLBACK_KIND, NotificationTypeDef, NotifyChannel,
    PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{PartyId, Subject, approvals};
use wl_notify::broadcast_run::{Batching, Ran, run_due, run_due_with, run_one};
use wl_notify::permissions::MARKETING_SEND;
use wl_notify::{Broadcast, BroadcastStatus, SuppressionReason, suppressions};

use support::{Booted, Recorded, boot_providers, record};

const OFFER: &str = "campaign.offer";
const MAIL_OFFER: &str = "campaign.mail_offer";

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

/// 0: active, 1: ended, 2: a guard of the campaign refuses.
#[derive(Clone, Default)]
struct Switch(Arc<AtomicU8>);

impl BroadcastRequesters for Switch {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
        _at: renox::db::DateTime,
    ) -> RequesterFuture<'a, RequesterCheck> {
        let answer = match self.0.load(Ordering::SeqCst) {
            0 => RequesterCheck::Active,
            1 => RequesterCheck::Ended,
            _ => RequesterCheck::GuardFailed("permit".into()),
        };
        Box::pin(async move { Ok(answer) })
    }
}

fn ids(range: std::ops::RangeInclusive<i64>) -> Vec<PartyId> {
    range.map(PartyId).collect()
}

fn register_types() {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms])
            .template(NotifyChannel::Sms, "en", None, "Offer for you"),
    );
    notify::register_type(
        NotificationTypeDef::new(MAIL_OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some("Offer"), "Offer for you"),
    );
}

/// The app, with the segments below and the switch for the requester check:
/// segment 1 has 150 members of which 100 consent, segment 2 has 300 of which
/// 180 consent, segment 3 has 350 who all consent.
async fn boot(
    sender_id: Option<&'static str>,
) -> (
    Booted,
    Switch,
    Recorded<BroadcastCancelled>,
    Recorded<BroadcastCompleted>,
) {
    register_types();
    let switch = Switch::default();
    let cancelled = Recorded::new();
    let completed = Recorded::new();
    let (s, c, d) = (switch.clone(), cancelled.clone(), completed.clone());
    let t = boot_providers(
        Some(Arc::new(Everyone)),
        move |m| match sender_id {
            Some(id) => m.sms_sender_id(id),
            None => m,
        },
        move |app| {
            let lists = Lists(BTreeMap::from([
                (1, [ids(1..=100), ids(351..=400)].concat()),
                (2, [ids(1..=180), ids(351..=470)].concat()),
                (3, ids(1..=350)),
            ]));
            let segments: Arc<dyn Segments> = Arc::new(lists);
            let consents: Arc<dyn Consents> = Arc::new(Granted((1..=350).collect()));
            let requesters: Arc<dyn BroadcastRequesters> = Arc::new(s);
            let app = app.provide(segments).provide(consents).provide(requesters);
            record(record(app, &c), &d)
        },
    )
    .await;
    (t, switch, cancelled, completed)
}

fn request(
    type_key: &str,
    channel: NotifyChannel,
    segment: i64,
    by: i64,
    send_at: renox::db::DateTime,
) -> BroadcastRequest {
    BroadcastRequest::new(
        type_key,
        Subject::new("sales.campaign", 7),
        by,
        channel,
        SegmentId(segment),
        send_at,
    )
}

async fn stored(t: &Booted, id: BroadcastId) -> Broadcast {
    Broadcast::find_or_404(t.db(), id.0).await.unwrap()
}

fn later(t: &Booted) -> renox::db::DateTime {
    t.clock.now() + renox::chrono::Duration::hours(2)
}

async fn ask(t: &Booted, segment: i64, by: i64) -> BroadcastId {
    let req = request(OFFER, NotifyChannel::Sms, segment, by, later(t));
    notify::notifier(t.app.state())
        .broadcast(t.app.state(), req)
        .await
        .unwrap()
}

#[renox::test]
async fn at_the_send_time_the_recipients_get_one_message_each_and_it_completes_once() {
    let (t, _, _, completed) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;

    // Not due yet: nothing happens.
    let early = send_at - renox::chrono::Duration::minutes(1);
    assert_eq!(run_due(t.app.state(), early).await.unwrap(), 0);
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Scheduled);

    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Completed);
    assert!(b.started_at.is_some() && b.completed_at.is_some());
    assert_eq!(b.recipients_resolved, Some(100));
    assert_eq!(b.excluded.0.of("no_consent"), 50);
    assert_eq!(b.excluded.0.total(), 50);

    let messages = t.messages().await;
    assert_eq!(messages.len(), 100);
    let parties: BTreeSet<_> = messages.iter().filter_map(|m| m.party_id).collect();
    assert_eq!(parties.len(), 100);
    assert!(messages.iter().all(|m| m.broadcast_id == Some(id.0)));
    assert!(messages.iter().all(|m| m.subject() == id.subject()));
    assert!(messages.iter().all(|m| m.channel == NotifyChannel::Sms));

    let events = completed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].broadcast, id);
    assert_eq!(events[0].recipients, 100);
    assert_eq!(events[0].excluded.get("no_consent"), Some(&50));

    // A second run changes nothing.
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 0);
    assert_eq!(t.messages().await.len(), 100);
    assert_eq!(completed.all().len(), 1);
}

#[renox::test]
async fn a_run_that_stopped_after_40_is_finished_by_the_next_with_the_other_60() {
    let (t, _, _, completed) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;

    let crash = Batching {
        size: 40,
        max_batches: Some(1),
    };
    let ran = run_one(t.app.state(), id.0, send_at, crash).await.unwrap();
    assert_eq!(ran, Ran::Partial);
    assert_eq!(stored(&t, id).await.status, BroadcastStatus::Sending);
    assert_eq!(t.messages().await.len(), 40);
    assert!(completed.all().is_empty());

    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    let messages = t.messages().await;
    assert_eq!(messages.len(), 100);
    let parties: BTreeSet<_> = messages.iter().filter_map(|m| m.party_id).collect();
    assert_eq!(parties.len(), 100, "no recipient has two messages");
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Completed);
    assert_eq!(b.recipients_resolved, Some(100));
    assert_eq!(completed.all().len(), 1);

    assert_eq!(
        run_due_with(t.app.state(), send_at, Batching::default())
            .await
            .unwrap(),
        0
    );
    assert_eq!(t.messages().await.len(), 100);
    assert_eq!(completed.all().len(), 1);
}

#[renox::test]
async fn a_large_segment_goes_out_in_batches_of_100() {
    let (t, _, _, completed) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    t.user("Bilal", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    // Above the limit: approved by the second holder first.
    let request_id = stored(&t, id).await.approval_request_id.unwrap();
    let b_user = t.user("Bea", &[MARKETING_SEND]).await;
    approvals::approve(t.app.state(), request_id, &b_user, "")
        .await
        .unwrap();
    let send_at = stored(&t, id).await.send_at;
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    assert_eq!(t.messages().await.len(), 350);
    assert_eq!(completed.all()[0].recipients, 350);
}

#[renox::test]
async fn a_broadcast_waiting_for_approval_is_not_started() {
    let (t, _, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    t.user("Bilal", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 0);
    assert_eq!(
        stored(&t, id).await.status,
        BroadcastStatus::PendingApproval
    );
    assert!(t.messages().await.iter().all(|m| m.broadcast_id.is_none()));
}

#[renox::test]
async fn a_requester_that_ended_before_the_send_cancels_it_and_tells_the_requester() {
    let (t, switch, cancelled, completed) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    for answer in [1, 2] {
        switch.0.store(0, Ordering::SeqCst);
        let id = ask(&t, 1, a.id).await;
        let send_at = stored(&t, id).await.send_at;
        switch.0.store(answer, Ordering::SeqCst);
        assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);

        let b = stored(&t, id).await;
        assert_eq!(b.status, BroadcastStatus::Cancelled);
        assert_eq!(b.cancel_reason.as_deref(), Some("requester_guard_failed"));
        let queued = t.messages().await;
        assert!(queued.iter().all(|m| m.broadcast_id.is_none()));
        let events: Vec<_> = cancelled
            .all()
            .into_iter()
            .filter(|e| e.broadcast == id)
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].reason,
            BroadcastCancelReason::RequesterGuardFailed
        );
        assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 0);
    }
    assert!(completed.all().is_empty());
    let notices = t.notices(&a, FALLBACK_KIND).await;
    assert_eq!(notices.len(), 2);
    assert!(
        notices[0].contains("requester_guard_failed"),
        "{}",
        notices[0]
    );
}

#[renox::test]
async fn an_address_that_opted_out_after_the_request_is_counted_and_gets_no_message() {
    let (t, _, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    assert_eq!(stored(&t, id).await.estimate, 100);

    let mut tx = t.db().begin().await.unwrap();
    suppressions::add(
        &mut tx,
        NotifyChannel::Sms,
        "+971500000005",
        SuppressionReason::OptOut,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    run_due(t.app.state(), send_at).await.unwrap();
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Completed);
    assert_eq!(b.recipients_resolved, Some(99));
    assert_eq!(b.excluded.0.of("opted_out"), 1);
    assert_eq!(b.excluded.0.of("no_consent"), 50);
    assert!(
        t.messages()
            .await
            .iter()
            .all(|m| m.address.as_deref() != Some("+971500000005"))
    );
}
