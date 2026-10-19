//! Relationships between parties (wl-parties §3) and the directory's
//! `related_contacts`: the party side of AC7 of #57, with the real
//! directory and the real database.

mod support;

use renox::chrono::Days;
use renox::prelude::*;
use wl_kernel::notify::ContactPurpose;
use wl_kernel::parties::directory;
use wl_kernel::{PartyId, PartyKind};
use wl_parties::{
    CompanyDetails, NewParty, NewRelationship, Party, PersonDetails, Relationship,
    RelationshipType, create, relate,
};

use support::{Booted, boot_unseeded};

async fn person(t: &Booted, name: &str) -> Party {
    let new = NewParty {
        kind: PartyKind::Person,
        legal_name_en: name.into(),
        person: Some(PersonDetails::default()),
        source: "back_office".into(),
        ..Default::default()
    };
    create(t.app.state(), new).await.unwrap()
}

async fn company(t: &Booted, name: &str) -> Party {
    let new = NewParty {
        kind: PartyKind::Company,
        legal_name_en: name.into(),
        company: Some(CompanyDetails::default()),
        source: "back_office".into(),
        ..Default::default()
    };
    create(t.app.state(), new).await.unwrap()
}

fn link(from: &Party, to: &Party, position: Option<&str>, primary: bool) -> NewRelationship {
    NewRelationship {
        from: from.id,
        to: to.id,
        relation_type: RelationshipType::ContactOf,
        position: position.map(str::to_owned),
        primary,
        ..Default::default()
    }
}

async fn contacts_of(t: &Booted, of: &Party, purpose: &ContactPurpose) -> Vec<(i64, bool)> {
    directory(t.app.state())
        .related_contacts(t.db(), PartyId(of.id), purpose)
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.party.0, c.primary))
        .collect()
}

/// `relate` refuses a company as the from party of `contact_of` and
/// `guardian_of`, a missing party, and keeps one primary per (to party,
/// type, position).
#[renox::test]
async fn relate_needs_a_person_and_keeps_one_primary() {
    let t = boot_unseeded().await;
    let state = t.app.state();
    let blue_dunes = company(&t, "Blue Dunes LLC").await;
    let red_rock = company(&t, "Red Rock Trading LLC").await;
    let ali = person(&t, "Ali").await;
    let mona = person(&t, "Mona").await;

    let err = relate(state, link(&red_rock, &blue_dunes, Some("finance"), false))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)), "{err:?}");
    let guardian = NewRelationship {
        relation_type: RelationshipType::GuardianOf,
        ..link(&red_rock, &blue_dunes, None, false)
    };
    assert!(matches!(
        relate(state, guardian).await.unwrap_err(),
        Error::Validation(_)
    ));
    let missing = NewRelationship {
        to: 9999,
        ..link(&ali, &blue_dunes, Some("finance"), false)
    };
    assert!(matches!(
        relate(state, missing).await.unwrap_err(),
        Error::Validation(_)
    ));
    assert!(Relationship::query().get(t.db()).await.unwrap().is_empty());

    // A company may be a member of a group: the person rule is for
    // `contact_of` and `guardian_of` only.
    let group = NewRelationship {
        relation_type: RelationshipType::MemberOfGroup,
        ..link(&red_rock, &blue_dunes, None, false)
    };
    relate(state, group).await.unwrap();

    let first = relate(state, link(&ali, &blue_dunes, Some("finance"), true))
        .await
        .unwrap();
    let ops = relate(state, link(&ali, &blue_dunes, Some("operations"), true))
        .await
        .unwrap();
    assert!(first.is_primary && ops.is_primary);
    // A new primary in finance clears the old one, and only that one.
    let second = relate(state, link(&mona, &blue_dunes, Some("finance"), true))
        .await
        .unwrap();
    let db = t.db();
    let stored = |id: i64| async move { Relationship::find(db, id).await.unwrap().unwrap() };
    assert!(!stored(first.id).await.is_primary);
    assert!(stored(second.id).await.is_primary);
    assert!(stored(ops.id).await.is_primary, "another position is kept");
}

/// `related_contacts` returns only the active relationships of that party,
/// relation and position, primary first, then by start date, then by id.
#[renox::test]
async fn related_contacts_are_active_of_that_party_and_in_order() {
    let t = boot_unseeded().await;
    let state = t.app.state();
    let today = wl_kernel::documents::today();
    let day = Days::new(1);
    let blue_dunes = company(&t, "Blue Dunes LLC").await;
    let red_rock = company(&t, "Red Rock Trading LLC").await;
    let ali = person(&t, "Ali").await;
    let mona = person(&t, "Mona").await;
    let nour = person(&t, "Nour").await;
    let omar = person(&t, "Omar").await;
    let hana = person(&t, "Hana").await;
    let zaid = person(&t, "Zaid").await;
    let later = person(&t, "Layla").await;
    let student = person(&t, "Student").await;

    // Created in the order Mona, Nour, Ali: the primary comes first, then
    // the older start date, then the id.
    let mut mona_link = link(&mona, &blue_dunes, Some("finance"), false);
    mona_link.valid_from = Some(today - day);
    relate(state, mona_link).await.unwrap();
    relate(state, link(&nour, &blue_dunes, Some("finance"), false))
        .await
        .unwrap();
    relate(state, link(&ali, &blue_dunes, Some("finance"), true))
        .await
        .unwrap();
    // Not returned: another position, another company, ended yesterday,
    // not started yet, and a guardian link.
    relate(state, link(&omar, &blue_dunes, Some("operations"), true))
        .await
        .unwrap();
    relate(state, link(&zaid, &red_rock, Some("finance"), true))
        .await
        .unwrap();
    let mut ended = link(&hana, &blue_dunes, Some("finance"), false);
    ended.valid_from = Some(today - day - day);
    ended.valid_until = Some(today - day);
    relate(state, ended).await.unwrap();
    let mut future = link(&later, &blue_dunes, Some("finance"), false);
    future.valid_from = Some(today + day);
    relate(state, future).await.unwrap();
    let guardian = NewRelationship {
        relation_type: RelationshipType::GuardianOf,
        ..link(&omar, &blue_dunes, None, true)
    };
    relate(state, guardian).await.unwrap();

    let finance = ContactPurpose::contact_of("finance");
    assert_eq!(
        contacts_of(&t, &blue_dunes, &finance).await,
        [(ali.id, true), (mona.id, false), (nour.id, false)]
    );
    // Without a position, any position of `contact_of` matches.
    let any = ContactPurpose {
        position: None,
        ..finance.clone()
    };
    let got = contacts_of(&t, &blue_dunes, &any).await;
    assert!(got.contains(&(omar.id, true)) && got.contains(&(ali.id, true)));
    assert!(
        !got.iter()
            .any(|(id, _)| [zaid.id, hana.id, later.id].contains(id))
    );
    assert_eq!(
        contacts_of(&t, &blue_dunes, &ContactPurpose::guardian_of()).await,
        [(omar.id, true)]
    );
    assert_eq!(
        contacts_of(&t, &red_rock, &finance).await,
        [(zaid.id, true)]
    );
    // A party nobody is related to.
    assert!(contacts_of(&t, &student, &finance).await.is_empty());
}
