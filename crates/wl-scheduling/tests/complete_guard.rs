//! The vertical's guard on `complete` (wl-scheduling §4.1 `complete`; 05
//! `STK.LOT.01`): a refusing guard blocks `service::complete` with its text
//! and leaves the booking as it was; a booking that has not been checked in is
//! never asked; the answer belongs to one booking.

mod support;

use renox::chrono::TimeDelta;
use wl_scheduling::service::{self, Actor};
use wl_scheduling::{Booking, BookingStatus, Refusal};

use support::{Booked, boot, in_hours, in_minutes, now};

fn staff() -> Actor {
    Actor::staff(1)
}

/// A booking of the clinic service, checked in and started.
async fn started(t: &Booked, doctor_name: &str) -> Booking {
    let doctor = t.doctor(doctor_name).await;
    let service = t
        .service_with("visit", &[&doctor], |s| {
            s.check_in_required = true;
            s.track_in_progress = true;
        })
        .await;
    let booking = t.booking(&service, &[&doctor], in_minutes(30)).await;
    service::check_in(t.state(), booking.id, staff(), now())
        .await
        .expect("checked in");
    service::start(t.state(), booking.id, staff(), now())
        .await
        .expect("started");
    t.reload(booking.id).await
}

/// A refusing guard blocks the completion with its text: the booking stays
/// `in_progress`, nothing is emitted; the refusal of one booking does not
/// touch another; once the guard allows, the booking completes.
#[renox::test]
async fn a_refusing_guard_blocks_the_completion() {
    let t = boot().await;
    let booking = started(&t, "Dr A").await;
    let other = started(&t, "Dr B").await;
    assert_eq!(booking.status, BookingStatus::InProgress);
    t.gate.refuse(booking.id, "Lot required: set the lot of filler.");
    let at = in_hours(3);

    let refused = service::complete(t.state(), booking.id, staff(), at).await;
    let Err(Refusal::Invalid(text)) = refused else {
        panic!("the completion is refused: {refused:?}");
    };
    assert_eq!(text, "Lot required: set the lot of filler.");
    let after = t.reload(booking.id).await;
    assert_eq!(after.status, BookingStatus::InProgress);
    assert!(after.completed_at.is_none());
    assert!(t.events.completed.all().is_empty());
    let names: Vec<_> = t
        .history(booking.id)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert!(!names.contains(&"complete".to_owned()), "{names:?}");

    // Another booking is another answer.
    let done = service::complete(t.state(), other.id, staff(), at)
        .await
        .expect("the other booking completes");
    assert_eq!(done.status, BookingStatus::Completed);
    assert_eq!(t.events.completed.all().len(), 1);

    // The guard lets it through: the booking completes.
    t.gate.allow(booking.id);
    let done = service::complete(t.state(), booking.id, staff(), at + TimeDelta::minutes(1))
        .await
        .expect("allowed now");
    assert_eq!(done.status, BookingStatus::Completed);
    assert_eq!(t.events.completed.all().len(), 2);
    assert_eq!(t.gate.asked(), [booking.id, other.id, booking.id]);
}

/// A booking that was not checked in (a service with no check-in) is never
/// put to the guard, whatever the guard would answer.
#[renox::test]
async fn a_booking_without_a_check_in_is_never_asked() {
    let t = boot().await;
    t.gate.refuse_all("Never to be read");
    let doctor = t.doctor("Dr A").await;
    let plain = t.service(&doctor).await;
    let booking = t.booking(&plain, &[&doctor], in_hours(2)).await;

    let done = service::complete(t.state(), booking.id, staff(), in_hours(3))
        .await
        .expect("a plain booking completes");
    assert_eq!(done.status, BookingStatus::Completed);
    assert!(t.gate.asked().is_empty(), "{:?}", t.gate.asked());
}
