//! Acceptance checks of sending a marketing broadcast to a segment (#61 part
//! 8): what the parts' own tests leave open, from the request to the page.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastCompleted, BroadcastId, BroadcastRequest, BroadcastRequesters, RequesterCheck,
    RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, NotificationTypeDef, NotifyChannel, PartyContactPoint,
    PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{COMPANY_ADMIN_ROLE, PartyId, Subject, approvals};
use wl_notify::broadcast_run::run_due;
use wl_notify::permissions::MARKETING_SEND;
use wl_notify::{Broadcast, BroadcastStatus, MessageStatus, SuppressionReason, suppressions};

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
#[derive(Clone)]
struct Granted(Arc<Mutex<BTreeSet<i64>>>);

impl Granted {
    fn withdraw_now(&self, party: i64) {
        self.0.lock().unwrap().remove(&party);
    }
}

impl Consents for Granted {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let yes = purpose == "marketing" && self.0.lock().unwrap().contains(&party.0);
        Box::pin(async move { Ok(yes) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
        _source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        let had = self.0.lock().unwrap().remove(&party.0);
        Box::pin(async move { Ok(had) })
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
async fn boot(sender_id: Option<&'static str>) -> (Booted, Granted, Recorded<BroadcastCompleted>) {
    register_types();
    let switch = Switch::default();
    let completed = Recorded::new();
    let granted = Granted(Arc::new(Mutex::new((1..=350).collect())));
    let (s, g, d) = (switch.clone(), granted.clone(), completed.clone());
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
            let consents: Arc<dyn Consents> = Arc::new(g);
            let requesters: Arc<dyn BroadcastRequesters> = Arc::new(s);
            let app = app.provide(segments).provide(consents).provide(requesters);
            record(app, &d)
        },
    )
    .await;
    (t, granted, completed)
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
async fn the_requester_cannot_approve_even_as_admin_and_a_second_holder_can() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let b_user = t.user("Bilal", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    let request_id = stored(&t, id).await.approval_request_id.unwrap();
    // The requester is an admin too: still refused as the requester.
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    a.assign_role(t.db(), COMPANY_ADMIN_ROLE).await.unwrap();
    assert!(
        approvals::approve(t.app.state(), request_id, &a, "")
            .await
            .is_err()
    );
    assert_eq!(
        stored(&t, id).await.status,
        BroadcastStatus::PendingApproval
    );
    approvals::approve(t.app.state(), request_id, &b_user, "")
        .await
        .unwrap();
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Scheduled);
    assert_eq!(b.approved_by, Some(b_user.id));
}

#[renox::test]
async fn consent_withdrawn_between_the_request_and_the_send_is_counted() {
    let (t, granted, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    assert_eq!(stored(&t, id).await.estimate, 100);
    let send_at = stored(&t, id).await.send_at;

    granted.withdraw_now(3);
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
    assert_eq!(b.recipients_resolved, Some(98));
    assert_eq!(b.excluded.0.of("no_consent"), 51);
    assert_eq!(b.excluded.0.of("opted_out"), 1);
    let messages = t.messages().await;
    assert_eq!(messages.len(), 98);
    assert!(messages.iter().all(|m| m.party_id != Some(3)));
}

#[renox::test]
async fn consent_withdrawn_after_the_start_suppresses_the_queued_message() {
    let (t, granted, completed) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;

    run_due(t.app.state(), send_at).await.unwrap();
    assert_eq!(completed.all().len(), 1);
    assert_eq!(t.messages().await.len(), 100);

    granted.withdraw_now(7);
    t.run_jobs().await;
    let messages = t.messages().await;
    let seven = messages.iter().find(|m| m.party_id == Some(7)).unwrap();
    assert_eq!(seven.status, MessageStatus::Suppressed);
    assert_eq!(seven.suppress_reason.as_deref(), Some("no_consent"));
    assert!(seven.broadcast_id.is_some());
    let suppressed = messages
        .iter()
        .filter(|m| m.status == MessageStatus::Suppressed)
        .count();
    assert_eq!(suppressed, 1);
}

#[renox::test]
async fn the_finished_broadcast_page_shows_counts_and_results_but_no_names() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let holder = t.user("Hana", &[MARKETING_SEND]).await;
    let nobody = t.user("Nadia", &["notifications.preferences.manage"]).await;
    let id = ask(&t, 1, a.id).await;
    let send_at = stored(&t, id).await.send_at;
    run_due(t.app.state(), send_at).await.unwrap();
    t.run_jobs().await;

    let b = stored(&t, id).await;
    let counts = renox::serde_json::to_string(&b.excluded.0).unwrap();
    assert!(counts.contains("no_consent"));

    let path = format!("/notify/broadcasts/{id}");
    for user in [&a, &holder] {
        t.app.acting_as(user);
        t.app
            .get(&path)
            .await
            .assert_ok()
            .assert_see("Completed")
            .assert_see("Estimated recipients")
            .assert_see("100")
            .assert_see("50")
            .assert_dont_see("+9715");
    }
    t.app.acting_as(&nobody);
    t.app.get(&path).await.assert_forbidden();
}
