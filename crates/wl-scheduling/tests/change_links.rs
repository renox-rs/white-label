//! The links in a message about a change the business made (#171, BL-SCH-04):
//! *accept* keeps the booking, *pick another time* lists the nearest free
//! times and moves the booking, also after the reschedule cut-off.

mod support;

use std::time::Duration;

use renox::prelude::*;
use wl_kernel::documents::dubai_text;
use wl_scheduling::pages::local_text;
use wl_scheduling::slots;
use wl_scheduling::{Booking, BookingStatus, links};

use support::{boot, in_hours};

const HOUR: Duration = Duration::from_secs(3600);

/// *Accept* changes nothing: the booking stays as it is and the page says so.
#[renox::test]
async fn accepting_the_change_changes_nothing() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(30)).await;
    let url = links::accept_url(t.state(), booking.id, HOUR).unwrap();

    t.app
        .get(&t.path_of(&url))
        .await
        .assert_ok()
        .assert_see("stays as it is");
    let same = t.reload(booking.id).await;
    assert_eq!(same.status, BookingStatus::Booked);
    assert_eq!(same.starts_at, booking.starts_at);
    assert!(t.events.rescheduled.all().is_empty());
    assert!(t.events.cancelled.all().is_empty());
}

/// A link whose signature is altered moves nothing.
#[renox::test]
async fn an_altered_link_is_refused() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(30)).await;
    let url = links::reschedule_url(t.state(), booking.id, HOUR).unwrap();
    let forged = t.path_of(&url).replace("signature=", "signature=0");
    let later = local_text(booking.starts_at + renox::chrono::TimeDelta::hours(2));

    t.app.get(&forged).await.assert_forbidden();
    t.app
        .htmx()
        .post(&forged, &[("starts_at", later.as_str())])
        .await
        .assert_forbidden();
    assert_eq!(t.reload(booking.id).await.starts_at, booking.starts_at);
}

/// *Pick another time* lists the nearest free times of the same resource;
/// picking one moves the booking, 3 hours before its start (cut-off 24
/// hours), re-checks the slot, and clears the conflict.
#[renox::test]
async fn picking_another_time_moves_the_booking_and_clears_the_conflict() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(3)).await;
    Booking::query()
        .where_(Booking::ID.eq(booking.id))
        .update(t.db(), &[("conflict_block", &true)])
        .await
        .unwrap();
    let nearest = slots::nearest(
        t.state(),
        &service,
        std::slice::from_ref(&doctor),
        booking.starts_at,
        support::now(),
        slots::SHOWN,
    )
    .await
    .unwrap();
    assert!(!nearest.is_empty(), "the doctor is free later");
    let pick = nearest[0].clone();
    let url = links::reschedule_url(t.state(), booking.id, HOUR).unwrap();
    let path = t.path_of(&url);

    let page = t.app.get(&path).await;
    page.assert_ok();
    for slot in &nearest {
        page.assert_see(&dubai_text(slot.start));
    }

    // The ordinary party reschedule is refused after the cut-off; this link
    // is not.
    t.app
        .htmx()
        .post(&path, &[("starts_at", local_text(pick.start).as_str())])
        .await
        .assert_ok()
        .assert_see("is moved");
    let moved = t.reload(booking.id).await;
    assert_eq!(moved.status, BookingStatus::Booked);
    assert_eq!(moved.starts_at, pick.start);
    assert!(!moved.has_conflict());
    let events = t.events.rescheduled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].old_start, booking.starts_at);
    assert_eq!(events[0].start, pick.start);
}

/// A time somebody else took meanwhile is refused with the slot just taken.
#[renox::test]
async fn a_time_taken_meanwhile_is_refused() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(3)).await;
    let later = booking.starts_at + renox::chrono::TimeDelta::hours(2);
    t.booking(&service, &[&doctor], later).await;
    let url = links::reschedule_url(t.state(), booking.id, HOUR).unwrap();

    t.app
        .htmx()
        .post(
            &t.path_of(&url),
            &[("starts_at", local_text(later).as_str())],
        )
        .await
        .assert_status(409);
    assert_eq!(t.reload(booking.id).await.starts_at, booking.starts_at);
}
