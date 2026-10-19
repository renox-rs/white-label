//! Opting out of marketing (#61 part 6): the footer of a broadcast message,
//! the `AD-` sender ID, the signed unsubscribe page and the STOP reply.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::broadcast::{
    BroadcastId, BroadcastRequest, BroadcastRequesters, RequesterCheck, RequesterFuture,
};
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, NotificationTypeDef, NotifyChannel, PartyContactPoint,
    PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_kernel::segments::{SegmentFuture, SegmentId, Segments};
use wl_kernel::{PartyId, Subject};
use wl_notify::broadcast_run::run_due;
use wl_notify::permissions::MARKETING_SEND;
use wl_notify::{ReplyOutcome, SuppressionReason, handle_reply, opt_out, suppressions};

use support::{Booted, boot_providers};

const SMS_OFFER: &str = "campaign.sms_offer";
const MAIL_OFFER: &str = "campaign.mail_offer";

/// Parties 1 and 2 are active with a mobile and an e-mail; party 2 writes
/// Arabic.
struct Two;

impl PartyDirectory for Two {
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
        let point = |id, kind, address: String| PartyContactPoint {
            id,
            kind,
            address,
            primary: true,
            verified: true,
            valid: true,
        };
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: if party.0 == 2 { "ar" } else { "en" }.into(),
                preferred_channel: None,
                points: vec![
                    point(party.0, ContactKind::Mobile, format!("+9715{:08}", party.0)),
                    point(
                        100 + party.0,
                        ContactKind::Email,
                        format!("P{}@Example.com", party.0),
                    ),
                ],
            }))
        })
    }
}

struct Pair;

impl Segments for Pair {
    fn members<'a>(
        &'a self,
        _db: &'a Db,
        _segment: SegmentId,
        _at: renox::db::DateTime,
    ) -> SegmentFuture<'a, Vec<PartyId>> {
        Box::pin(async { Ok(vec![PartyId(1), PartyId(2)]) })
    }
}

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

/// Everyone consents; the withdrawals are recorded as (party, channel,
/// source) and a second withdrawal of the same consent answers `false`.
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<(i64, NotifyChannel, String)>>>);

impl Recording {
    fn withdrawals(&self) -> Vec<(i64, NotifyChannel, String)> {
        self.0.lock().unwrap().clone()
    }
}

impl Consents for Recording {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let withdrawn = self
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(p, c, _)| *p == party.0 && *c == channel);
        let yes = purpose == "marketing" && !withdrawn;
        Box::pin(async move { Ok(yes) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        _purpose: &'a str,
        channel: NotifyChannel,
        source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        let mut all = self.0.lock().unwrap();
        let first = !all.iter().any(|(p, c, _)| *p == party.0 && *c == channel);
        if first {
            all.push((party.0, channel, source.to_owned()));
        }
        Box::pin(async move { Ok(first) })
    }
}

async fn boot() -> (Booted, Recording) {
    notify::register_type(
        NotificationTypeDef::new(SMS_OFFER, "sales")
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
    let consents = Recording::default();
    let handle = consents.clone();
    let t = boot_providers(
        Some(Arc::new(Two)),
        |m| m.sms_sender_id("AD-SCHOOL"),
        move |app| {
            let segments: Arc<dyn Segments> = Arc::new(Pair);
            let consents: Arc<dyn Consents> = Arc::new(consents);
            let requesters: Arc<dyn BroadcastRequesters> = Arc::new(Active);
            app.provide(segments).provide(consents).provide(requesters)
        },
    )
    .await;
    (t, handle)
}

/// Requests a broadcast of `type_key` and runs it to its end, sending the
/// messages.
async fn broadcast(t: &Booted, type_key: &str, channel: NotifyChannel) -> BroadcastId {
    let user = t.user("Aisha", &[MARKETING_SEND]).await;
    let send_at = t.clock.now() + renox::chrono::Duration::hours(2);
    let request = BroadcastRequest::new(
        type_key,
        Subject::new("sales.campaign", 7),
        user.id,
        channel,
        SegmentId(1),
        send_at,
    );
    let id = notify::notifier(t.app.state())
        .broadcast(t.app.state(), request)
        .await
        .unwrap();
    assert_eq!(run_due(t.app.state(), send_at).await.unwrap(), 1);
    t.run_jobs().await;
    id
}

/// The token in the unsubscribe link at the end of `body`.
fn token_in(body: &str) -> String {
    let link = body.rsplit(' ').next().unwrap();
    link.rsplit('/').next().unwrap().to_owned()
}

#[renox::test]
async fn an_email_carries_the_brand_and_a_link_that_opts_out_once_confirmed() {
    let (t, consents) = boot().await;
    broadcast(&t, MAIL_OFFER, NotifyChannel::Email).await;

    let sent = t.email.sent();
    assert_eq!(sent.len(), 2);
    let brand = t.app.state().config.name.clone();
    for mail in &sent {
        assert!(mail.body.starts_with("Offer for you"), "{}", mail.body);
        assert!(mail.body.contains(&brand), "{}", mail.body);
        assert!(mail.body.contains("/unsubscribe/"), "{}", mail.body);
    }
    let first = sent.iter().find(|m| m.address == "p1@example.com").unwrap();
    let path = format!("/unsubscribe/{}", token_in(&first.body));

    // Opening the link asks; it changes nothing.
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_view("wl-notify/unsubscribe.html")
        .assert_see("Unsubscribe from marketing emails");
    assert!(consents.withdrawals().is_empty());

    // Confirming withdraws the e-mail consent and suppresses the address.
    t.app
        .htmx()
        .post(&path, &[])
        .await
        .assert_ok()
        .assert_see("You are unsubscribed");
    assert_eq!(
        consents.withdrawals(),
        [(1, NotifyChannel::Email, "unsubscribe_link".to_owned())]
    );
    assert_eq!(
        suppressions::reason_for(t.db(), NotifyChannel::Email, "P1@example.com")
            .await
            .unwrap(),
        Some(SuppressionReason::OptOut)
    );
    assert!(
        !suppressions::is_suppressed(t.db(), NotifyChannel::Email, "p2@example.com")
            .await
            .unwrap()
    );

    // Again: the same result and nothing more.
    t.app
        .htmx()
        .post(&path, &[])
        .await
        .assert_ok()
        .assert_see("You are unsubscribed");
    assert_eq!(consents.withdrawals().len(), 1);
}

#[renox::test]
async fn a_tampered_token_changes_nothing() {
    let (t, consents) = boot().await;
    broadcast(&t, MAIL_OFFER, NotifyChannel::Email).await;
    let mail = t.email.sent().remove(0);
    let token = token_in(&mail.body);

    let mut changed = token.clone();
    let last = changed.pop().unwrap();
    changed.push(if last == '0' { '1' } else { '0' });
    for bad in [changed, "1".to_owned(), "zz".to_owned(), String::new()] {
        if bad.is_empty() {
            continue;
        }
        let path = format!("/unsubscribe/{bad}");
        t.app
            .get(&path)
            .await
            .assert_ok()
            .assert_see("Link not valid");
        t.app
            .htmx()
            .post(&path, &[])
            .await
            .assert_ok()
            .assert_see("Link not valid");
    }
    assert!(consents.withdrawals().is_empty());
    assert!(
        !suppressions::is_suppressed(t.db(), NotifyChannel::Email, "p1@example.com")
            .await
            .unwrap()
    );
    assert!(opt_out::message_of(t.app.state(), &token).is_some());
}

#[renox::test]
async fn an_sms_uses_the_ad_sender_id_and_ends_with_the_stop_line_in_both_languages() {
    let (t, _) = boot().await;
    broadcast(&t, SMS_OFFER, NotifyChannel::Sms).await;

    let sent = t.sms.sent();
    assert_eq!(sent.len(), 2);
    let brand = t.app.state().config.name.clone();
    for sms in &sent {
        assert_eq!(sms.sender_id.as_deref(), Some("AD-SCHOOL"));
        assert!(sms.body.contains(&brand), "{}", sms.body);
        assert!(sms.body.contains("STOP"), "{}", sms.body);
    }
    let english = sent.iter().find(|m| m.language == "en").unwrap();
    assert!(english.body.ends_with("Reply STOP to opt out"));
    let arabic = sent.iter().find(|m| m.language == "ar").unwrap();
    assert!(arabic.body.ends_with("STOP"), "{}", arabic.body);
    assert!(!arabic.body.contains("Reply"), "{}", arabic.body);
}

#[renox::test]
async fn the_stop_reply_in_any_case_withdraws_the_sms_consent_and_suppresses_the_number() {
    let (t, consents) = boot().await;
    broadcast(&t, SMS_OFFER, NotifyChannel::Sms).await;
    let state = t.app.state();

    // Not the keyword: ignored.
    let ignored = handle_reply(state, NotifyChannel::Sms, "+971500000001", "stop it").await;
    assert_eq!(ignored.unwrap(), ReplyOutcome::Ignored);
    assert!(consents.withdrawals().is_empty());

    for body in ["stop", "Stop", "STOP "] {
        let outcome = handle_reply(state, NotifyChannel::Sms, " +971500000001 ", body)
            .await
            .unwrap();
        assert_eq!(outcome, ReplyOutcome::OptedOut(1), "{body:?}");
    }
    assert_eq!(
        consents.withdrawals(),
        [(1, NotifyChannel::Sms, "sms_stop".to_owned())]
    );
    assert_eq!(
        suppressions::reason_for(t.db(), NotifyChannel::Sms, "+971500000001")
            .await
            .unwrap(),
        Some(SuppressionReason::OptOut)
    );
    // The other number is untouched.
    assert!(
        !suppressions::is_suppressed(t.db(), NotifyChannel::Sms, "+971500000002")
            .await
            .unwrap()
    );
}
