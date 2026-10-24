//! Bookings and resources in the search index and the Related panel
//! (BL-AGT-11): a booking is found by its number as it is made, its
//! document follows its transitions, a resource has its document after the
//! rebuild, and the relations are produced (a booking's party and
//! resources, a resource's bookings); the booking page loads its panel.

mod support;

use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access};
use wl_scheduling::permissions as perm;
use wl_scheduling::resource::RESOURCE_SUBJECT_TYPE;
use wl_scheduling::service::{self, Actor};

use support::{boot, in_minutes, now};

fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

#[renox::test]
async fn a_booking_is_found_by_its_number_and_shows_its_party_and_resources() {
    let t = boot().await;
    let doctor = t.doctor("Dr Amal").await;
    let consultation = t.service(&doctor).await;
    let booking = t.booking(&consultation, &[&doctor], in_minutes(60)).await;
    let subject = Subject::new(BOOKING_SUBJECT_TYPE, booking.id);

    // Made, the booking is indexed and found by its number typed loosely.
    let document = search::document_of(t.db(), &subject)
        .await
        .unwrap()
        .expect("the booking is indexed as it is made");
    assert_eq!(document.title, booking.number);
    assert_eq!(document.status.as_deref(), Some("booked"));
    let typed = booking.number.to_lowercase().replace('-', " ");
    let found = search::lookup_identifier(t.db(), &typed).await.unwrap();
    assert!(found.iter().any(|m| m.subject == subject));

    // `confirm` (a transition) writes the new status.
    service::confirm(t.state(), booking.id, Actor::party(None), now())
        .await
        .unwrap();
    let document = search::document_of(t.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.status.as_deref(), Some("confirmed"));

    // The resource, saved as a fixture, comes in with the rebuild.
    search::rebuild_type(t.db(), RESOURCE_SUBJECT_TYPE)
        .await
        .unwrap();
    let resource_subject = Subject::new(RESOURCE_SUBJECT_TYPE, doctor.id);
    let document = search::document_of(t.db(), &resource_subject)
        .await
        .unwrap()
        .expect("the resource is indexed");
    assert_eq!(document.title, "Dr Amal");

    let access = Access::with(
        1,
        [
            (perm::VIEW.to_owned(), Reach::everything()),
            ("parties.view".to_owned(), Reach::everything()),
        ],
    );
    let groups = related::related(t.db(), &access, &subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "resources"), Some(1));
    // The party is the parties module's: without it here, the group shows
    // the party by its id to holders of `parties.view`.
    if wl_kernel::search::searchable_type(PARTY_SUBJECT_TYPE).is_none() {
        assert_eq!(count(&groups, "party"), Some(1));
    }
    let groups = related::related(t.db(), &access, &resource_subject, 1)
        .await
        .unwrap();
    assert_eq!(count(&groups, "bookings"), Some(1));

    // The booking page loads its panel.
    let desk = t.front_desk().await;
    t.app.acting_as(&desk);
    t.app
        .get(&format!("/scheduling/bookings/{}", booking.id))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/scheduling.booking:{}/panel\"",
            booking.id
        ));
}
