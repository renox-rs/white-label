//! Asking for a broadcast (#61 part 4): the checks on the request, the
//! estimate, `scheduled` or `pending_approval` by the limit, the approval by
//! a second holder and the cancel announced once.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastCancelReason, BroadcastCancelled, BroadcastId, BroadcastRequest, BroadcastRequesters,
    RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, FALLBACK_KIND, NotificationTypeDef, NotifyChannel,
    PartyContactPoint, PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{PartyId, Subject, approvals};
use wl_notify::permissions::MARKETING_SEND;
use wl_notify::{Broadcast, BroadcastStatus};

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
async fn boot(sender_id: Option<&'static str>) -> (Booted, Switch, Recorded<BroadcastCancelled>) {
    register_types();
    let switch = Switch::default();
    let cancelled = Recorded::new();
    let (s, c) = (switch.clone(), cancelled.clone());
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
            record(app, &c)
        },
    )
    .await;
    (t, switch, cancelled)
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

fn is_422<T: std::fmt::Debug>(result: Result<T>) -> bool {
    matches!(result, Err(Error::Status(code, _)) if code == StatusCode::UNPROCESSABLE_ENTITY)
}

fn later(t: &Booted) -> renox::db::DateTime {
    t.clock.now() + renox::chrono::Duration::hours(2)
}

#[renox::test]
async fn a_request_within_the_limit_is_scheduled_with_its_estimate() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    for (segment, estimate) in [(1, 100), (2, 180)] {
        let req = request(OFFER, NotifyChannel::Sms, segment, a.id, later(&t));
        let id = notify::notifier(t.app.state())
            .broadcast(t.app.state(), req)
            .await
            .unwrap();
        let b = stored(&t, id).await;
        assert_eq!(b.status, BroadcastStatus::Scheduled);
        assert_eq!(b.estimate, estimate);
        assert_eq!(b.requested_by, a.id);
        assert_eq!(b.requester_type, "sales.campaign");
        assert_eq!(b.requester_id, 7);
        assert_eq!(b.segment_id, segment);
        assert_eq!(b.approval_request_id, None);
    }
}

#[renox::test]
async fn above_the_limit_a_second_holder_approves() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let b_user = t.user("Bilal", &[MARKETING_SEND]).await;
    let c = t.user("Chen", &["notifications.preferences.manage"]).await;
    let req = request(OFFER, NotifyChannel::Sms, 3, a.id, later(&t));
    let id = notify::notifier(t.app.state())
        .broadcast(t.app.state(), req)
        .await
        .unwrap();
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::PendingApproval);
    assert_eq!(b.estimate, 350);
    let request_id = b.approval_request_id.expect("an approval request");

    // The approvers were told; the requester and the other holder were not.
    assert_eq!(t.notices(&b_user, "approval-requested").await.len(), 1);
    assert!(t.notices(&a, "approval-requested").await.is_empty());

    for refused in [&a, &c] {
        assert!(
            approvals::approve(t.app.state(), request_id, refused, "")
                .await
                .is_err()
        );
    }
    approvals::approve(t.app.state(), request_id, &b_user, "")
        .await
        .unwrap();
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Scheduled);
    assert_eq!(b.approved_by, Some(b_user.id));
}

#[renox::test]
async fn a_rejection_cancels_with_the_comment_and_tells_the_requester_once() {
    let (t, _, cancelled) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let b_user = t.user("Bilal", &[MARKETING_SEND]).await;
    let req = request(OFFER, NotifyChannel::Sms, 3, a.id, later(&t));
    let id = notify::notifier(t.app.state())
        .broadcast(t.app.state(), req)
        .await
        .unwrap();
    let request_id = stored(&t, id).await.approval_request_id.unwrap();
    approvals::reject(t.app.state(), request_id, &b_user, "wrong audience")
        .await
        .unwrap();

    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("wrong audience"));
    let events = cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].broadcast, id);
    assert_eq!(events[0].requester, Subject::new("sales.campaign", 7));
    assert_eq!(events[0].reason, BroadcastCancelReason::Rejected);
    let notices = t.notices(&a, FALLBACK_KIND).await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("wrong audience"), "{}", notices[0]);
}

#[renox::test]
async fn a_withdrawn_request_cancels_the_broadcast_as_cancelled() {
    let (t, _, cancelled) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    t.user("Bilal", &[MARKETING_SEND]).await;
    let req = request(OFFER, NotifyChannel::Sms, 3, a.id, later(&t));
    let id = notify::notifier(t.app.state())
        .broadcast(t.app.state(), req)
        .await
        .unwrap();
    let request_id = stored(&t, id).await.approval_request_id.unwrap();
    approvals::withdraw(t.app.state(), request_id, &a)
        .await
        .unwrap();
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("cancelled"));
    assert_eq!(cancelled.all().len(), 1);
    assert_eq!(cancelled.all()[0].reason, BroadcastCancelReason::Cancelled);
}

#[renox::test]
async fn a_request_that_breaks_a_rule_is_refused() {
    let (t, switch, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let ask = |req: BroadcastRequest| {
        let state = t.app.state().clone();
        async move { notify::notifier(&state).broadcast(&state, req).await }
    };
    let fine = request(OFFER, NotifyChannel::Sms, 1, a.id, later(&t));

    // No named user, or one that does not exist.
    for by in [0, -1, 99_999] {
        assert!(is_422(
            ask(BroadcastRequest {
                requested_by: by,
                ..fine.clone()
            })
            .await
        ));
    }
    // A type that is not marketing, an unknown one, a channel it lacks.
    assert!(is_422(
        ask(BroadcastRequest {
            type_key: support::REMINDER.into(),
            ..fine.clone()
        })
        .await
    ));
    assert!(is_422(
        ask(BroadcastRequest {
            type_key: "no.such".into(),
            ..fine.clone()
        })
        .await
    ));
    assert!(is_422(
        ask(BroadcastRequest {
            channel: NotifyChannel::Email,
            ..fine.clone()
        })
        .await
    ));
    assert!(is_422(
        ask(request(MAIL_OFFER, NotifyChannel::Sms, 1, a.id, later(&t))).await
    ));
    // An ended requester, and a guard of its module that refuses.
    for answer in [1, 2] {
        switch.0.store(answer, Ordering::SeqCst);
        assert!(is_422(ask(fine.clone()).await));
    }
    switch.0.store(0, Ordering::SeqCst);
    ask(fine).await.unwrap();
    assert_eq!(Broadcast::query().get(t.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn a_marketing_sms_without_an_ad_sender_id_is_refused_but_email_is_not() {
    let (t, _, _) = boot(None).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let sms = request(OFFER, NotifyChannel::Sms, 1, a.id, later(&t));
    assert!(is_422(
        notify::notifier(t.app.state())
            .broadcast(t.app.state(), sms)
            .await
    ));
    let mail = request(MAIL_OFFER, NotifyChannel::Email, 1, a.id, later(&t));
    notify::notifier(t.app.state())
        .broadcast(t.app.state(), mail)
        .await
        .unwrap();
    let (t, _, _) = boot(Some("SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let sms = request(OFFER, NotifyChannel::Sms, 1, a.id, later(&t));
    assert!(is_422(
        notify::notifier(t.app.state())
            .broadcast(t.app.state(), sms)
            .await
    ));
}

#[renox::test]
async fn without_a_requesters_provider_every_request_is_refused() {
    register_types();
    let t = boot_providers(
        Some(Arc::new(Everyone)),
        |m| m.sms_sender_id("AD-SCHOOL"),
        |app| app,
    )
    .await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let req = request(OFFER, NotifyChannel::Sms, 1, a.id, later(&t));
    assert!(is_422(
        notify::notifier(t.app.state())
            .broadcast(t.app.state(), req)
            .await
    ));
}
