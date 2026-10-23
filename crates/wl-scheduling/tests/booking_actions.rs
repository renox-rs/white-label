//! The booking actions a vertical has (wl-kernel `BookingActions`): booking a
//! service for the first free slot from a day (08 `AUT.RUL.04`, a fleet
//! service that falls due) and cancelling a booking as the system (08: a
//! fleet contract that ended).

mod support;

use wl_kernel::scheduling::{BookingActions, ServiceBooking};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::BookingStatus;
use wl_scheduling::actions::SchedulingActions;

use support::{PATIENT, boot, in_hours};

/// The service is booked for the party at the first free slot from the day
/// asked: a second request for the same time goes to the next day.
#[renox::test]
async fn a_service_is_booked_for_the_first_free_slot_from_a_day() {
    let t = boot().await;
    let bay = t.doctor("Bay 1").await;
    t.service_with("periodic_service", &[&bay], |_| {}).await;
    let from = in_hours(30);
    let request = ServiceBooking::new(PATIENT, "periodic_service", from)
        .for_subject(Subject::new("auto.vehicle", 3))
        .note("Due at 60,000 km");

    let first = SchedulingActions
        .book_service(t.state(), request.clone())
        .await
        .unwrap();
    let second = SchedulingActions
        .book_service(t.state(), request)
        .await
        .unwrap();
    assert_ne!(first, second);
    let one = t.reload(first.0).await;
    let two = t.reload(second.0).await;
    assert_eq!(one.status, BookingStatus::Booked);
    assert_eq!(one.party_id, PATIENT.0);
    assert!((one.starts_at - from).num_seconds().abs() < 2);
    assert_eq!(one.subject_type.as_deref(), Some("auto.vehicle"));
    assert_eq!(one.subject_id, Some(3));
    assert_eq!(one.notes.as_deref(), Some("Due at 60,000 km"));
    assert!(two.starts_at > one.starts_at, "the bay is taken then");
}

/// A service that does not exist is refused.
#[renox::test]
async fn an_unknown_service_is_refused() {
    let t = boot().await;
    let refused = SchedulingActions
        .book_service(
            t.state(),
            ServiceBooking::new(PartyId(5), "nothing", in_hours(30)),
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("no service"), "{refused}");
}

/// A booking is cancelled as the system with the reason code; one that was
/// cancelled already is left as it is.
#[renox::test]
async fn a_booking_is_cancelled_as_the_system() {
    let t = boot().await;
    let bay = t.doctor("Bay 1").await;
    t.service_with("periodic_service", &[&bay], |_| {}).await;
    let booking = SchedulingActions
        .book_service(
            t.state(),
            ServiceBooking::new(PATIENT, "periodic_service", in_hours(30)),
        )
        .await
        .unwrap();

    assert!(
        SchedulingActions
            .cancel_booking(t.state(), booking, "resource_unavailable")
            .await
            .unwrap()
    );
    let cancelled = t.reload(booking.0).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert!(
        !SchedulingActions
            .cancel_booking(t.state(), booking, "resource_unavailable")
            .await
            .unwrap()
    );
}
