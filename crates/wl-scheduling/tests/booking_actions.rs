//! The booking actions a vertical has (wl-kernel `BookingActions`): booking a
//! service for the first free slot from a day (08 `AUT.RUL.04`, a fleet
//! service that falls due) and cancelling a booking as the system (08: a
//! fleet contract that ended).

mod support;

use wl_kernel::scheduling::{BookingActions, ServiceBooking, WorkDone};
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
    t.service_with("periodic_service", &[&bay], |service| {
        service.check_in_required = true;
    })
    .await;
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

/// The booking says when it starts (08 `BL-AUT-12`: the bookings after the last
/// day of a contract); one that does not exist has no start.
#[renox::test]
async fn a_booking_says_when_it_starts() {
    let t = boot().await;
    let bay = t.doctor("Bay 1").await;
    t.service_with("periodic_service", &[&bay], |_| {}).await;
    let from = in_hours(30);
    let booking = SchedulingActions
        .book_service(
            t.state(),
            ServiceBooking::new(PATIENT, "periodic_service", from),
        )
        .await
        .unwrap();
    let start = SchedulingActions
        .starts_at(t.state(), booking)
        .await
        .unwrap()
        .expect("a start");
    assert!((start - from).num_seconds().abs() < 2);
    assert!(
        SchedulingActions
            .starts_at(t.state(), wl_kernel::scheduling::BookingId(9_999))
            .await
            .unwrap()
            .is_none()
    );
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

/// After the check-in a booking is cancelled for a staff user, once the
/// record opened at check-in has its work done known; the system alone cannot
/// (`SCH.ACL.06`), and the work done pending holds it back.
#[renox::test]
async fn a_checked_in_booking_is_cancelled_for_a_staff_user_once_its_work_done_is_known() {
    let t = boot().await;
    let bay = t.doctor("Bay 1").await;
    t.service_with("periodic_service", &[&bay], |service| {
        service.check_in_required = true;
    })
    .await;
    let booking = SchedulingActions
        .book_service(
            t.state(),
            ServiceBooking::new(PATIENT, "periodic_service", in_hours(1)),
        )
        .await
        .unwrap();
    wl_scheduling::service::check_in(
        t.state(),
        booking.0,
        wl_scheduling::service::Actor::staff(1),
        support::now(),
    )
    .await
    .expect("the party is checked in");
    assert_eq!(t.reload(booking.0).await.status, BookingStatus::CheckedIn);

    // The system alone is refused after the check-in.
    assert!(
        !SchedulingActions
            .cancel_booking(t.state(), booking, "walked_out")
            .await
            .unwrap()
    );
    // The record has not said what was done: the booking stays.
    t.work.set(booking.0, WorkDone::Pending);
    assert!(
        !SchedulingActions
            .cancel_booking_by_staff(t.state(), booking, "walked_out", 1)
            .await
            .unwrap()
    );
    assert_eq!(t.reload(booking.0).await.status, BookingStatus::CheckedIn);

    t.work.set(booking.0, WorkDone::Nothing);
    assert!(
        SchedulingActions
            .cancel_booking_by_staff(t.state(), booking, "walked_out", 1)
            .await
            .unwrap()
    );
    assert_eq!(t.reload(booking.0).await.status, BookingStatus::Cancelled);
    assert!(
        !SchedulingActions
            .cancel_booking_by_staff(t.state(), booking, "walked_out", 1)
            .await
            .unwrap(),
        "a booking that was cancelled is left as it is"
    );
}
