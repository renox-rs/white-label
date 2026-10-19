//! The contact points on the party page (#58 part 3): the "Contact needs
//! updating" banner, the status column, and the edit and verify actions
//! behind `parties.update`.

mod support;

use renox::prelude::*;
use wl_parties::{ContactPoint, ContactStatus, ContactType, Party, permissions as perm};

use support::{Booted, boot};

const AL_NOOR: &str = "Al Noor Trading LLC";

/// The party's primary email, marked invalid after a hard bounce.
async fn invalid_email(t: &Booted, party: &Party) -> ContactPoint {
    let mut point = ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(party.id))
        .get(t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.contact_type == ContactType::Email)
        .expect("an email contact point");
    point.status = ContactStatus::Invalid;
    point.invalid_since = Some(renox::db::now());
    point.invalid_reason = Some("hard_bounce".to_owned());
    point.save(t.db()).await.unwrap();
    point
}

async fn reload(t: &Booted, point: &ContactPoint) -> ContactPoint {
    ContactPoint::find(t.db(), point.id).await.unwrap().unwrap()
}

#[renox::test]
async fn the_page_asks_staff_to_update_an_invalid_contact() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let page = format!("/parties/{}", party.id);
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;

    t.app.acting_as(&viewer);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_dont_see("Contact needs updating");

    let point = invalid_email(&t, &party).await;
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Contact needs updating")
        .assert_see(&point.value)
        .assert_see("Invalid: Hard bounce")
        .assert_dont_see("Update contact");

    t.app.acting_as(&staff);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Contact needs updating")
        .assert_see("Update contact")
        .assert_see(&format!(
            "/parties/{}/contact-points/{}",
            party.id, point.id
        ));
}

#[renox::test]
async fn editing_the_address_makes_the_point_valid_again() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = invalid_email(&t, &party).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let route = format!("/parties/{}/contact-points/{}", party.id, point.id);

    t.app
        .htmx()
        .post(&route, &[("value", "not-an-email"), ("label", "")])
        .await
        .assert_status(422);
    assert_eq!(reload(&t, &point).await.status, ContactStatus::Invalid);

    t.app
        .htmx()
        .post(
            &route,
            &[("value", "ali.r@bluedunes.ae"), ("label", "work")],
        )
        .await
        .assert_ok();
    let after = reload(&t, &point).await;
    assert_eq!(after.status, ContactStatus::Valid);
    assert_eq!(after.invalid_since, None);
    assert_eq!(after.invalid_reason, None);
    assert_eq!(after.is_primary, point.is_primary);
    t.app
        .get(&format!("/parties/{}", party.id))
        .await
        .assert_ok()
        .assert_dont_see("Contact needs updating");
}

#[renox::test]
async fn verifying_makes_the_point_valid_and_is_staff_only() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = invalid_email(&t, &party).await;
    let route = format!("/parties/{}/contact-points/{}/verify", party.id, point.id);

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app.htmx().post(&route, &[]).await.assert_status(403);
    assert_eq!(reload(&t, &point).await.status, ContactStatus::Invalid);

    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    t.app.htmx().post(&route, &[]).await.assert_ok();
    let after = reload(&t, &point).await;
    assert_eq!(after.status, ContactStatus::Valid);
    assert!(after.verified_at.is_some());
}

#[renox::test]
async fn a_point_of_another_party_is_not_found() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let point = invalid_email(&t, &party).await;
    let other = Party::query()
        .where_(Party::ID.ne(party.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("another party");
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);

    t.app
        .htmx()
        .post(
            &format!("/parties/{}/contact-points/{}", other.id, point.id),
            &[("value", "x@y.ae")],
        )
        .await
        .assert_status(404);
    t.app
        .htmx()
        .post(
            &format!("/parties/{}/contact-points/{}/verify", other.id, point.id),
            &[],
        )
        .await
        .assert_status(404);
    assert_eq!(reload(&t, &point).await.status, ContactStatus::Invalid);
}
