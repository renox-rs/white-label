//! The broadcast recipient resolver and the consent guard of `send` (#61 part
//! 3): the exclusion reasons and counts, the primary contact point, guardians
//! and the `no_consent` suppression of a marketing message.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, ContactKind, NotificationTypeDef, NotifyChannel, PartyContactPoint,
    PartyContacts,
};
use wl_kernel::parties::{PartyDirectory, PartyFuture, PartyStatus, PartySummary};
use wl_notify::broadcast_recipients::{BLOCKED_PARTY, NO_CONSENT, OPTED_OUT, resolve_members};
use wl_notify::{MessageStatus, Suppression, SuppressionReason};

use support::{BLUE_DUNES, boot};

/// An in-memory directory: statuses, contact points and guardians.
#[derive(Default)]
struct Directory {
    status: BTreeMap<i64, PartyStatus>,
    points: BTreeMap<i64, Vec<PartyContactPoint>>,
    guardians: BTreeMap<i64, Vec<PartyId>>,
}

fn mobile(id: i64, address: &str, primary: bool) -> PartyContactPoint {
    PartyContactPoint {
        id,
        kind: ContactKind::Mobile,
        address: address.to_owned(),
        primary,
        verified: true,
        valid: true,
    }
}

impl PartyDirectory for Directory {
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

    fn status<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Option<PartyStatus>> {
        let status = self.status.get(&party.0).copied();
        Box::pin(async move { Ok(status) })
    }

    fn contacts<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
    ) -> PartyFuture<'a, Option<PartyContacts>> {
        let points = self.points.get(&party.0).cloned().unwrap_or_default();
        Box::pin(async move {
            Ok(Some(PartyContacts {
                party,
                language: "ar".into(),
                preferred_channel: None,
                points,
            }))
        })
    }

    fn guardians<'a>(&'a self, _db: &'a Db, party: PartyId) -> PartyFuture<'a, Vec<PartyId>> {
        let found = self.guardians.get(&party.0).cloned().unwrap_or_default();
        Box::pin(async move { Ok(found) })
    }
}

/// Consents for the parties in the set, on every channel.
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

fn directory() -> Directory {
    let mut d = Directory::default();
    for id in 1..=12 {
        d.status.insert(id, PartyStatus::Active);
        d.points.insert(
            id,
            vec![mobile(id * 10, &format!("+9715000000{id:02}"), true)],
        );
    }
    d
}

fn ids(list: &[i64]) -> Vec<PartyId> {
    list.iter().map(|&i| PartyId(i)).collect()
}

#[renox::test]
async fn members_without_consent_are_counted_and_the_others_kept() {
    let t = boot().await;
    let d = directory();
    let granted = Granted((1..=3).collect());
    let r = resolve_members(
        t.db(),
        &d,
        &granted,
        &ids(&[1, 2, 3, 4, 5, 2]),
        NotifyChannel::Sms,
    )
    .await
    .unwrap();
    assert_eq!(r.members, 5, "a repeated member counts once");
    assert_eq!(r.recipients.len(), 3);
    assert_eq!(r.excluded.of(NO_CONSENT), 2);
    assert_eq!(r.excluded.total(), 2);
    assert_eq!(r.recipients[0].address, "+971500000001");
    assert_eq!(r.recipients[0].language, "ar");
}

#[renox::test]
async fn blocked_archived_merged_and_unknown_parties_are_left_out() {
    let t = boot().await;
    let mut d = directory();
    d.status.insert(2, PartyStatus::Blocked);
    d.status.insert(3, PartyStatus::Archived);
    d.status.insert(4, PartyStatus::Merged);
    d.status.remove(&5);
    let granted = Granted((1..=12).collect());
    let r = resolve_members(
        t.db(),
        &d,
        &granted,
        &ids(&[1, 2, 3, 4, 5]),
        NotifyChannel::Sms,
    )
    .await
    .unwrap();
    assert_eq!(r.recipients.len(), 1);
    assert_eq!(r.excluded.of(BLOCKED_PARTY), 4);
}

#[renox::test]
async fn two_mobile_points_give_one_recipient_at_the_primary() {
    let t = boot().await;
    let mut d = directory();
    d.points.insert(
        1,
        vec![
            mobile(11, "+971500000111", false),
            mobile(12, "+971500000222", true),
        ],
    );
    d.points.insert(2, Vec::new());
    let granted = Granted((1..=2).collect());
    let r = resolve_members(t.db(), &d, &granted, &ids(&[1, 2]), NotifyChannel::Sms)
        .await
        .unwrap();
    assert_eq!(r.recipients.len(), 1);
    assert_eq!(r.recipients[0].address, "+971500000222");
    assert_eq!(r.recipients[0].contact_point, 12);
    assert_eq!(r.excluded.of("no_contact_point"), 1);
}

#[renox::test]
async fn the_suppression_list_names_its_reason_before_consent() {
    let t = boot().await;
    let d = directory();
    for (address, reason) in [
        ("+971500000001", SuppressionReason::OptOut),
        ("+971500000002", SuppressionReason::HardBounce),
    ] {
        Suppression::create(
            t.db(),
            Suppression {
                address: address.to_owned(),
                channel: NotifyChannel::Sms,
                reason,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    // Party 3 has no consent, parties 1 and 2 are suppressed: suppression wins.
    let granted = Granted(BTreeSet::from([1, 2]));
    let r = resolve_members(t.db(), &d, &granted, &ids(&[1, 2, 3]), NotifyChannel::Sms)
        .await
        .unwrap();
    assert!(r.recipients.is_empty());
    assert_eq!(r.excluded.of(OPTED_OUT), 1);
    assert_eq!(r.excluded.of("suppressed"), 1);
    assert_eq!(r.excluded.of(NO_CONSENT), 1);
}

#[renox::test]
async fn a_minor_goes_to_the_consenting_guardians_once() {
    let t = boot().await;
    let mut d = directory();
    // 1 has guardians 6 (consents) and 7 (does not); 2 has only 7; 3 has 6 too
    // and 6 is also a member itself.
    d.guardians.insert(1, ids(&[6, 7]));
    d.guardians.insert(2, ids(&[7]));
    d.guardians.insert(3, ids(&[6]));
    let granted = Granted(BTreeSet::from([6]));
    let r = resolve_members(
        t.db(),
        &d,
        &granted,
        &ids(&[1, 2, 3, 6]),
        NotifyChannel::Sms,
    )
    .await
    .unwrap();
    let parties: Vec<_> = r.recipients.iter().map(|x| x.party).collect();
    assert_eq!(parties, ids(&[6]), "guardian 6 is reached once");
    assert_eq!(r.recipients[0].on_behalf_of, Some(PartyId(1)));
    assert_eq!(r.excluded.of(NO_CONSENT), 1);
    assert_eq!(r.excluded.total(), 1);
}

#[renox::test]
async fn a_marketing_message_without_consent_is_suppressed_at_send() {
    let t = boot().await;
    notify::register_type(
        NotificationTypeDef::new("campaign.offer_test", "campaigns")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(NotifyChannel::Email, "en", Some("Offer"), "A new offer."),
    );
    let ids = t.notify_party("campaign.offer_test", BLUE_DUNES).await;
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("no_consent"));
    assert!(t.email.sent().is_empty());
}
