//! The staff waiting list: list, add, offer a freed slot, accept, remove
//! (BL-SCH-03).

mod support;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_scheduling::pages::local_text;
use wl_scheduling::waiting::{WaitingEntry, WaitingStatus};

use support::{OTHER_PATIENT, PATIENT, boot, in_hours};

/// The desk adds a party; the list shows the entry, a viewer may look but not
/// change anything, and a window that ends before it starts is refused in
/// words.
#[renox::test]
async fn the_desk_adds_a_party_and_a_viewer_only_looks() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let viewer = t.viewer().await;
    let (party, svc, from, until) = (
        PATIENT.0.to_string(),
        service.id.to_string(),
        local_text(in_hours(48)),
        local_text(in_hours(72)),
    );
    let form = [
        ("party", party.as_str()),
        ("service", svc.as_str()),
        ("window_from", from.as_str()),
        ("window_until", until.as_str()),
    ];

    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post("/scheduling/waiting-list", &form)
        .await
        .assert_forbidden();
    t.app
        .get("/scheduling/waiting-list")
        .await
        .assert_ok()
        .assert_see("Nobody is waiting");

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            "/scheduling/waiting-list",
            &[
                ("party", party.as_str()),
                ("service", svc.as_str()),
                ("window_from", until.as_str()),
                ("window_until", from.as_str()),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post("/scheduling/waiting-list", &form)
        .await
        .assert_status(303);
    let entry = WaitingEntry::find_or_404(t.db(), 1).await.unwrap();
    assert_eq!(entry.status, WaitingStatus::Waiting);
    assert_eq!(entry.subject_id, Some(PATIENT.0));
    t.app
        .get("/scheduling/waiting-list")
        .await
        .assert_ok()
        .assert_see("Consultation")
        .assert_see("Waiting");
}

/// The entry's page lists the free times that match it; offering one holds
/// it against other bookers, the party's yes books it, and a time outside the
/// entry's window is refused.
#[renox::test]
async fn staff_offer_a_matching_time_and_book_it_when_the_party_says_yes() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let from = in_hours(48);
    let until = from + TimeDelta::hours(3);
    let entry = wl_scheduling::waiting::join(
        t.state(),
        wl_scheduling::waiting::NewEntry::new(PATIENT, service.id)
            .with(doctor.id)
            .between(from, until),
    )
    .await
    .unwrap();
    t.app.acting_as(&desk);
    let page = format!("/scheduling/waiting-list/{}", entry.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Free times that match")
        .assert_see("Dr Hana");

    let (doctor_id, inside, outside) = (
        doctor.id.to_string(),
        local_text(from + TimeDelta::hours(1)),
        local_text(until + TimeDelta::hours(5)),
    );
    t.app
        .htmx()
        .post(
            &format!("{page}/offer"),
            &[
                ("resource", doctor_id.as_str()),
                ("starts_at", outside.as_str()),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("{page}/offer"),
            &[
                ("resource", doctor_id.as_str()),
                ("starts_at", inside.as_str()),
            ],
        )
        .await
        .assert_status(303);
    let offered = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(offered.status, WaitingStatus::Offered);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Book the offered time")
        .assert_see("held until");

    // The held time is not offered to anyone else.
    let taken = wl_scheduling::service::book(
        t.state(),
        wl_scheduling::NewBooking::new(
            OTHER_PATIENT,
            service.id,
            &[doctor.id],
            offered.offered_from.unwrap(),
        ),
        wl_scheduling::Actor::staff(1),
        renox::db::now(),
    )
    .await;
    assert!(taken.is_err());

    t.app
        .htmx()
        .post(&format!("{page}/accept"), &[("x", "y")])
        .await
        .assert_status(303);
    let booked = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(booked.status, WaitingStatus::Booked);
    let booking = t.reload(booked.booking_id.unwrap()).await;
    assert_eq!(booking.party_id, PATIENT.0);
}

/// Removing an entry withdraws it and frees a held time.
#[renox::test]
async fn removing_an_entry_frees_the_held_time() {
    let t = boot().await;
    let doctor = t.doctor("Dr Hana").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let start = in_hours(48);
    let entry = wl_scheduling::waiting::join(
        t.state(),
        wl_scheduling::waiting::NewEntry::new(PATIENT, service.id),
    )
    .await
    .unwrap();
    wl_scheduling::waiting::offer(
        t.state(),
        entry.id,
        doctor.id,
        start,
        start + TimeDelta::minutes(30),
        wl_scheduling::Actor::staff(desk.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/waiting-list/{}/withdraw", entry.id),
            &[("x", "y")],
        )
        .await
        .assert_status(303);
    let gone = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(gone.status, WaitingStatus::Withdrawn);
    t.booking(&service, &[&doctor], start).await;
}
