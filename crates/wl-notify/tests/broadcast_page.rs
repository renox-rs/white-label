//! The broadcasts pages (#61 part 7): the list, the detail page with its
//! approval card and counts, and the cancel button.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastCancelled, BroadcastRequest, BroadcastRequesters, RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, NotificationTypeDef, NotifyChannel, PartyContactPoint,
    PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{PartyId, Subject};
use wl_notify::permissions::{LOG_VIEW, MARKETING_SEND};
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

fn request(segment: i64, by: i64, send_at: renox::db::DateTime) -> BroadcastRequest {
    BroadcastRequest::new(
        OFFER,
        Subject::new("sales.campaign", 7),
        by,
        NotifyChannel::Sms,
        SegmentId(segment),
        send_at,
    )
}

fn later(t: &Booted) -> renox::db::DateTime {
    t.clock.now() + renox::chrono::Duration::hours(2)
}

async fn ask(t: &Booted, segment: i64, by: i64) -> i64 {
    notify::notifier(t.app.state())
        .broadcast(t.app.state(), request(segment, by, later(t)))
        .await
        .unwrap()
        .0
}

async fn stored(t: &Booted, id: i64) -> Broadcast {
    Broadcast::find_or_404(t.db(), id).await.unwrap()
}

/// The detail page answers 200 for the requester and for a holder, 403 for
/// others and 404 for an unknown id; it shows the estimate, the rule text,
/// the template and the counts but no party names.
#[renox::test]
async fn the_detail_page_is_for_the_requester_and_holders_only() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let holder = t.user("Hana", &[MARKETING_SEND]).await;
    let nobody = t.user("Nadia", &[LOG_VIEW]).await;
    let id = ask(&t, 1, a.id).await;
    let path = format!("/notify/broadcasts/{id}");

    t.app.acting_as(&a);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_view("wl-notify/broadcast.html")
        .assert_see("Scheduled")
        .assert_see("Estimated recipients")
        .assert_see("100")
        .assert_see("Segment 1 of sales.campaign #7")
        .assert_see("Offer for you")
        .assert_see("Cancel broadcast")
        .assert_dont_see("+9715");

    t.app.acting_as(&holder);
    t.app.get(&path).await.assert_ok();
    t.app.get("/notify/broadcasts/999").await.assert_not_found();

    t.app.acting_as(&nobody);
    t.app.get(&path).await.assert_forbidden();
    t.app
        .htmx()
        .post(&format!("{path}/cancel"), &[])
        .await
        .assert_status(403);
}

/// A broadcast above the limit shows the kernel approval card.
#[renox::test]
async fn an_over_limit_broadcast_shows_the_approval_card() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 3, a.id).await;
    assert_eq!(
        stored(&t, id).await.status,
        BroadcastStatus::PendingApproval
    );
    t.app.acting_as(&a);
    t.app
        .get(&format!("/notify/broadcasts/{id}"))
        .await
        .assert_ok()
        .assert_see("Pending approval")
        .assert_see("350");
}

/// The list shows a holder every broadcast and anyone else only their own.
#[renox::test]
async fn the_list_shows_holders_all_and_others_their_own() {
    let (t, _, _) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let holder = t.user("Hana", &[MARKETING_SEND]).await;
    let nobody = t.user("Nadia", &[LOG_VIEW]).await;
    let first = ask(&t, 1, a.id).await;
    let second = ask(&t, 2, holder.id).await;

    t.app.acting_as(&holder);
    t.app
        .get("/notify/broadcasts")
        .await
        .assert_ok()
        .assert_view("wl-notify/broadcasts.html")
        .assert_see(&format!("/notify/broadcasts/{first}"))
        .assert_see(&format!("/notify/broadcasts/{second}"));

    t.app.acting_as(&nobody);
    t.app
        .get("/notify/broadcasts")
        .await
        .assert_ok()
        .assert_see("No broadcasts yet");
}

/// Cancelling from the page cancels a scheduled broadcast with the reason
/// and tells the requester once; the cancel button is gone afterwards.
#[renox::test]
async fn cancelling_from_the_page_cancels_the_broadcast() {
    let (t, _, cancelled) = boot(Some("AD-SCHOOL")).await;
    let a = t.user("Aisha", &[MARKETING_SEND]).await;
    let id = ask(&t, 1, a.id).await;
    let path = format!("/notify/broadcasts/{id}");

    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&format!("{path}/cancel"), &[("reason", "wrong audience")])
        .await
        .assert_status(303);
    let b = stored(&t, id).await;
    assert_eq!(b.status, BroadcastStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("wrong audience"));
    assert_eq!(cancelled.all().len(), 1);

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Cancelled")
        .assert_see("wrong audience")
        .assert_dont_see("Cancel broadcast");

    // A second cancel is refused by the machine.
    t.app
        .htmx()
        .post(&format!("{path}/cancel"), &[])
        .await
        .assert_status(422);
    assert_eq!(cancelled.all().len(), 1);
}
