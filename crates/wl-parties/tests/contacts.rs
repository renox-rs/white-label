//! The status of a contact point (#58 AC4, AC7): marked invalid through the
//! kernel's directory, valid again by editing the address or verifying it,
//! and never chosen by the notifier while invalid. The page is in
//! `contact_page.rs`.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::notify::ContactKind;
use wl_kernel::parties::InvalidatedContact;
use wl_parties::{ContactPoint, ContactStatus, ContactType, Party, permissions as perm};

use support::{Booted, boot};

const AL_NOOR: &str = "Al Noor Trading LLC";

async fn email_of(t: &Booted, party: &Party) -> ContactPoint {
    ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.id))
        .get(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.contact_type == ContactType::Email)
        .expect("an email contact point")
}

async fn reload(t: &Booted, point: &ContactPoint) -> ContactPoint {
    ContactPoint::find(t.db(), point.id).await.unwrap().unwrap()
}

/// Whether the kernel's view of the party's contacts says `point` is valid.
async fn valid_for_notify(t: &Booted, party: &Party, point: &ContactPoint) -> bool {
    let directory = wl_kernel::parties::directory(t.app.state());
    let contacts = directory
        .contacts(t.db(), PartyId(party.id))
        .await
        .unwrap()
        .expect("the party has contacts");
    contacts
        .points
        .iter()
        .find(|p| p.id == point.id)
        .expect("the point")
        .valid
}

/// A point goes from valid to invalid once, keeps its primary flag and its
/// row, keeps its first since and reason, and the notifier's view skips it.
#[renox::test]
async fn invalidating_changes_a_valid_point_once() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = email_of(&t, &party).await;
    assert_eq!(point.status, ContactStatus::Valid);
    assert!(valid_for_notify(&t, &party, &point).await);
    let directory = wl_kernel::parties::directory(t.app.state());

    let first = renox::db::now();
    let marked = directory
        .invalidate_contact(t.db(), point.id, first, "hard_bounce")
        .await
        .unwrap();
    assert_eq!(
        marked,
        Some(InvalidatedContact {
            party: PartyId(party.id),
            kind: ContactKind::Email
        })
    );
    let after = reload(&t, &point).await;
    assert_eq!(after.status, ContactStatus::Invalid);
    assert_eq!(after.invalid_reason.as_deref(), Some("hard_bounce"));
    assert!(after.invalid_since.is_some());
    assert_eq!(after.is_primary, point.is_primary);
    assert_eq!(after.value, point.value);
    assert!(!valid_for_notify(&t, &party, &point).await);

    // AC7: already invalid: nothing changes, first since and reason stay.
    let again = directory
        .invalidate_contact(t.db(), point.id, renox::db::now(), "invalid_number")
        .await
        .unwrap();
    assert_eq!(again, None);
    let kept = reload(&t, &point).await;
    assert_eq!(kept.invalid_reason.as_deref(), Some("hard_bounce"));
    assert_eq!(kept.invalid_since, after.invalid_since);

    // An unknown point is nothing.
    assert_eq!(
        directory
            .invalidate_contact(t.db(), 999_999, first, "hard_bounce")
            .await
            .unwrap(),
        None
    );
}

/// AC4: changing the address makes it valid and clears since and reason; a
/// label alone keeps the status; verifying sets `verified_at`.
#[renox::test]
async fn editing_and_verifying_reset_the_status() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = email_of(&t, &party).await;
    let directory = wl_kernel::parties::directory(t.app.state());
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let route = format!("/parties/{}/contact-points/{}", party.id, point.id);

    let marked = directory
        .invalidate_contact(t.db(), point.id, renox::db::now(), "hard_bounce")
        .await
        .unwrap();
    assert!(marked.is_some());
    t.app
        .htmx()
        .post(
            &route,
            &[("value", point.value.as_str()), ("label", "work")],
        )
        .await
        .assert_ok();
    let labelled = reload(&t, &point).await;
    assert_eq!(labelled.status, ContactStatus::Invalid);
    assert_eq!(labelled.label.as_deref(), Some("work"));
    assert!(labelled.invalid_since.is_some());

    t.app
        .htmx()
        .post(
            &route,
            &[("value", "ali.r@bluedunes.ae"), ("label", "work")],
        )
        .await
        .assert_ok();
    let edited = reload(&t, &point).await;
    assert_eq!(edited.value, "ali.r@bluedunes.ae");
    assert_eq!(edited.status, ContactStatus::Valid);
    assert_eq!(edited.invalid_since, None);
    assert_eq!(edited.invalid_reason, None);
    assert_eq!(edited.is_primary, point.is_primary);
    assert!(valid_for_notify(&t, &party, &point).await);

    // Invalid again, then verified: valid, `verified_at` set.
    let marked = directory
        .invalidate_contact(t.db(), point.id, renox::db::now(), "hard_bounce")
        .await
        .unwrap();
    assert!(marked.is_some());
    t.app
        .htmx()
        .post(&format!("{route}/verify"), &[])
        .await
        .assert_ok();
    let verified = reload(&t, &point).await;
    assert_eq!(verified.status, ContactStatus::Valid);
    assert_eq!(verified.invalid_reason, None);
    assert!(verified.verified_at.is_some());
}

/// AC4: a malformed e-mail is a 422 and a user without `parties.update` a
/// 403; neither changes the point.
#[renox::test]
async fn a_bad_edit_or_a_viewer_changes_nothing() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = email_of(&t, &party).await;
    let directory = wl_kernel::parties::directory(t.app.state());
    directory
        .invalidate_contact(t.db(), point.id, renox::db::now(), "hard_bounce")
        .await
        .unwrap();
    let route = format!("/parties/{}/contact-points/{}", party.id, point.id);

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(&route, &[("value", "ali.r@bluedunes.ae")])
        .await
        .assert_status(403);

    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    t.app
        .htmx()
        .post(&route, &[("value", "ali@")])
        .await
        .assert_status(422);

    let after = reload(&t, &point).await;
    assert_eq!(after.status, ContactStatus::Invalid);
    assert_eq!(after.value, point.value);
}
