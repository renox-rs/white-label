//! The booking lifecycle (wl-scheduling §4.1): book, request, accept,
//! decline, confirm, reschedule, check in, start, complete, no-show and
//! cancel, through the services and the staff pages.

mod support;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_scheduling::service::{self, Actor, CancelRequest, NewBooking};
use wl_scheduling::{BookingStatus, Refusal};

use support::{PATIENT, boot, in_hours, in_minutes, now};

fn staff() -> Actor {
    Actor::staff(1)
}

/// A booking is made `booked`, with a number, the slot held, an event and a
/// history row.
#[renox::test]
async fn staff_book_a_free_slot() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let booking = t.booking(&service, &[&doctor], start).await;
    assert_eq!(booking.status, BookingStatus::Booked);
    assert!(booking.number.starts_with("BKG-"), "{}", booking.number);
    assert_eq!(booking.starts_at, start);
    assert_eq!(booking.ends_at, start + TimeDelta::minutes(30));
    assert_eq!(booking.resource_ids(t.db()).await.unwrap(), [doctor.id]);
    assert_eq!(t.events.booked.all().len(), 1);
    assert_eq!(t.events.booked.all()[0].service_type, "consultation");
    assert_eq!(
        t.history(booking.id).await,
        [("book".to_owned(), "booked".to_owned())]
    );
}

/// The same resource cannot be booked twice at overlapping times; the next
/// slot is free.
#[renox::test]
async fn a_slot_is_not_booked_twice() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    t.booking(&service, &[&doctor], start).await;
    let overlap = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start + TimeDelta::minutes(15)),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(overlap, Err(Refusal::Taken { .. })), "{overlap:?}");
    t.booking(&service, &[&doctor], start + TimeDelta::minutes(30))
        .await;
}

/// A resource that is not open then, or is blocked, cannot be booked.
#[renox::test]
async fn a_closed_or_blocked_resource_is_refused() {
    let t = boot().await;
    let closed = t.resource("Dr C", wl_scheduling::ResourceType::Person).await;
    let service = t.service(&closed).await;
    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[closed.id], in_hours(48)),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(refused, Err(Refusal::Unavailable(_))), "{refused:?}");

    let open = t.doctor("Dr B").await;
    let service_b = t.service_with("follow_up", &[&open], |_| {}).await;
    let start = in_hours(72);
    let mut block = wl_scheduling::AvailabilityBlock::manual(
        open.id,
        start - TimeDelta::minutes(10),
        start + TimeDelta::hours(1),
        "sick",
    );
    block.save(t.db()).await.unwrap();
    let blocked = service::book(
        t.state(),
        NewBooking::new(PATIENT, service_b.id, &[open.id], start),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(blocked, Err(Refusal::Unavailable(_))), "{blocked:?}");
}

/// A booking in the past, or for a resource that does not deliver the
/// service, is refused.
#[renox::test]
async fn the_request_itself_must_be_valid() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let other = t.doctor("Dr B").await;
    let service = t.service(&doctor).await;
    let past = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], in_hours(-2)),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(past, Err(Refusal::Invalid(_))), "{past:?}");
    let wrong = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[other.id], in_hours(48)),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(wrong, Err(Refusal::Invalid(_))), "{wrong:?}");
}

/// A service that needs acceptance is requested; staff accept or decline.
#[renox::test]
async fn a_request_is_accepted_or_declined() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let start = in_hours(48);
    let booked = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start),
        Actor::party(None),
        now(),
    )
    .await;
    assert!(matches!(booked, Err(Refusal::Invalid(_))), "a party must request");

    let requested = service::request(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start).source("portal"),
        Actor::party(None),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(requested.status, BookingStatus::Requested);
    assert!(requested.hold_until.is_some());

    let accepted = service::accept(t.state(), requested.id, staff(), now())
        .await
        .unwrap();
    assert_eq!(accepted.status, BookingStatus::Booked);
    assert_eq!(accepted.hold_until, None);
    assert_eq!(t.events.booked.all().len(), 1);

    let later = service::request(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start + TimeDelta::hours(2)),
        Actor::party(None),
        now(),
    )
    .await
    .unwrap();
    let without_reason = service::decline(t.state(), later.id, " ", staff(), now()).await;
    assert!(without_reason.is_err());
    let declined = service::decline(t.state(), later.id, "No cover that day", staff(), now())
        .await
        .unwrap();
    assert_eq!(declined.status, BookingStatus::Cancelled);
    assert_eq!(declined.cancel_code.as_deref(), Some("other"));
    assert_eq!(declined.cancel_note.as_deref(), Some("No cover that day"));
    assert_eq!(t.events.released.all().len(), 1);

    // A service that needs no acceptance takes no request.
    let plain = t.service_with("plain", &[&doctor], |_| {}).await;
    let refused = service::request(
        t.state(),
        NewBooking::new(PATIENT, plain.id, &[doctor.id], start + TimeDelta::hours(5)),
        Actor::party(None),
        now(),
    )
    .await;
    assert!(matches!(refused, Err(Refusal::Invalid(_))), "{refused:?}");
}

/// The visit: confirm, check in, start, complete, each from the right state.
#[renox::test]
async fn a_visit_goes_from_booked_to_completed() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("visit", &[&doctor], |s| {
            s.check_in_required = true;
            s.track_in_progress = true;
        })
        .await;
    let start = in_minutes(30);
    let booking = t.booking(&service, &[&doctor], start).await;

    let confirmed = service::confirm(t.state(), booking.id, Actor::party(None), now())
        .await
        .unwrap();
    assert_eq!(confirmed.status, BookingStatus::Confirmed);
    assert!(confirmed.confirmed_at.is_some());

    // Not before check-in, and not before the start.
    let early = service::complete(t.state(), booking.id, staff(), now()).await;
    assert!(early.is_err());

    let arrived = service::check_in(t.state(), booking.id, staff(), now())
        .await
        .unwrap();
    assert_eq!(arrived.status, BookingStatus::CheckedIn);
    assert_eq!(t.events.checked_in.all().len(), 1);

    let running = service::start(t.state(), booking.id, staff(), now())
        .await
        .unwrap();
    assert_eq!(running.status, BookingStatus::InProgress);

    // Complete only once the start time has passed.
    let too_soon = service::complete(t.state(), booking.id, staff(), now()).await;
    assert!(too_soon.is_err());
    let at = start + TimeDelta::minutes(31);
    let done = service::complete(t.state(), booking.id, staff(), at)
        .await
        .unwrap();
    assert_eq!(done.status, BookingStatus::Completed);
    assert!(done.completed_at.is_some());
    assert_eq!(t.events.completed.all().len(), 1);
    assert_eq!(
        t.history(booking.id)
            .await
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["book", "confirm", "check_in", "start", "complete"]
    );
}

/// A service without check-in refuses `check_in` and `start`; one that
/// needs it refuses `complete` from `booked`.
#[renox::test]
async fn the_service_type_switches_the_steps_on_and_off() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let plain = t.service(&doctor).await;
    let booking = t.booking(&plain, &[&doctor], in_hours(2)).await;
    assert!(
        service::check_in(t.state(), booking.id, staff(), now())
            .await
            .is_err()
    );
    // Without check-in a booked booking completes once it has started.
    let done = service::complete(t.state(), booking.id, staff(), in_hours(3))
        .await
        .unwrap();
    assert_eq!(done.status, BookingStatus::Completed);

    let strict = t
        .service_with("strict", &[&doctor], |s| s.check_in_required = true)
        .await;
    let next = t.booking(&strict, &[&doctor], in_hours(5)).await;
    assert!(
        service::complete(t.state(), next.id, staff(), in_hours(6))
            .await
            .is_err()
    );
}

/// A no-show needs the grace period to pass, releases the slot and says
/// whether a fee applies.
#[renox::test]
async fn a_party_who_did_not_come_is_marked_no_show() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("deposit", &[&doctor], |s| s.deposit_amount = 20_000)
        .await;
    let start = in_hours(3);
    let booking = t.booking(&service, &[&doctor], start).await;
    let early = service::mark_no_show(t.state(), booking.id, staff(), start + TimeDelta::minutes(14)).await;
    assert!(early.is_err());
    let done = service::mark_no_show(t.state(), booking.id, staff(), start + TimeDelta::minutes(15))
        .await
        .unwrap();
    assert_eq!(done.status, BookingStatus::NoShow);
    let event = &t.events.no_show.all()[0];
    assert!(event.fee_applicable);
    assert_eq!(t.events.released.all().len(), 1);
    // The slot is free again.
    t.booking(&service, &[&doctor], start).await;
}

/// A conflicted booking is not marked no-show.
#[renox::test]
async fn a_conflicted_booking_is_not_a_no_show() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(3);
    let booking = t.booking(&service, &[&doctor], start).await;
    wl_scheduling::Booking::query()
        .where_(wl_scheduling::Booking::ID.eq(booking.id))
        .update(t.db(), &[("conflict_block", &true)])
        .await
        .unwrap();
    let refused =
        service::mark_no_show(t.state(), booking.id, staff(), start + TimeDelta::hours(1)).await;
    assert!(refused.is_err());
}

/// A cancellation needs a reason code, releases the slot and clears the
/// conflict; a cancelled booking cannot change again.
#[renox::test]
async fn a_cancelled_booking_releases_its_slot() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let booking = t.booking(&service, &[&doctor], start).await;

    let missing = service::cancel(t.state(), booking.id, CancelRequest::new(" "), staff(), now()).await;
    assert!(missing.is_err());
    let done = service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("site_closed"),
        staff(),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("site_closed"));
    assert_eq!(done.cancel_by.as_deref(), Some("staff"));
    assert_eq!(t.events.released.all().len(), 1);
    assert_eq!(t.events.released.all()[0].resources, [doctor.id]);

    let again = service::confirm(t.state(), booking.id, staff(), now()).await;
    assert!(again.is_err());
    // The slot is free again.
    t.booking(&service, &[&doctor], start).await;
}

/// A reschedule moves the booking and its slot; a party may do it only
/// before the cut-off.
#[renox::test]
async fn a_booking_is_rescheduled() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(72);
    let booking = t.booking(&service, &[&doctor], start).await;
    let new_start = in_hours(96);
    let moved = service::reschedule(t.state(), booking.id, new_start, None, Actor::party(None), now())
        .await
        .unwrap();
    assert_eq!(moved.status, BookingStatus::Booked);
    assert_eq!(moved.starts_at, new_start);
    assert_eq!(moved.ends_at, new_start + TimeDelta::minutes(30));
    let events = t.events.rescheduled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].old_start, start);
    assert_eq!(t.events.released.all().len(), 1);
    // The old slot is free, the new one is taken.
    t.booking(&service, &[&doctor], start).await;
    let taken = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], new_start),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(taken, Err(Refusal::Taken { .. })));

    // Inside the 24 h cut-off a party cannot, staff can.
    let soon = t.booking(&service, &[&doctor], in_hours(10)).await;
    let target = in_hours(30);
    let party =
        service::reschedule(t.state(), soon.id, target, None, Actor::party(None), now()).await;
    assert!(party.is_err());
    let staff_moved = service::reschedule(t.state(), soon.id, target, None, staff(), now())
        .await
        .unwrap();
    assert_eq!(staff_moved.starts_at, target);
}

/// A reschedule of a service that needs acceptance returns to `requested`.
#[renox::test]
async fn a_reschedule_returns_to_requested_when_acceptance_is_needed() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let booking = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], in_hours(72)),
        staff(),
        now(),
    )
    .await
    .unwrap();
    let moved = service::reschedule(t.state(), booking.id, in_hours(96), None, staff(), now())
        .await
        .unwrap();
    assert_eq!(moved.status, BookingStatus::Requested);
    assert!(moved.hold_until.is_some());
}

/// A service with two resources (a practitioner and a room) needs both free.
#[renox::test]
async fn every_required_resource_must_be_free() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let room = t.resource("Room 1", wl_scheduling::ResourceType::Room).await;
    t.open_all_week(&room).await;
    let procedure = t
        .service_with("procedure", &[&doctor, &room], |s| {
            s.required_types = "person,room".to_owned();
        })
        .await;
    let start = in_hours(48);
    let only_doctor = service::book(
        t.state(),
        NewBooking::new(PATIENT, procedure.id, &[doctor.id], start),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(only_doctor, Err(Refusal::Invalid(_))));
    t.booking(&procedure, &[&doctor, &room], start).await;
    // The room is busy: another doctor cannot use it then.
    let other = t.doctor("Dr B").await;
    wl_scheduling::resource::offer(t.db(), procedure.id, other.id)
        .await
        .unwrap();
    let busy = service::book(
        t.state(),
        NewBooking::new(PATIENT, procedure.id, &[other.id, room.id], start),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(busy, Err(Refusal::Taken { .. })), "{busy:?}");
}

/// The buffers of a service type count as occupied.
#[renox::test]
async fn buffers_keep_the_neighbouring_slot_busy() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("buffered", &[&doctor], |s| s.buffer_after_minutes = 10)
        .await;
    let start = in_hours(48);
    t.booking(&service, &[&doctor], start).await;
    let touching = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start + TimeDelta::minutes(30)),
        staff(),
        now(),
    )
    .await;
    assert!(matches!(touching, Err(Refusal::Taken { .. })));
    t.booking(&service, &[&doctor], start + TimeDelta::minutes(40))
        .await;
}

/// Prepaid bookings: the booking is a prepaid subject while it is open.
#[renox::test]
async fn an_open_booking_is_an_active_prepaid_subject() {
    use wl_kernel::payments::prepaid_subjects;
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let subjects = prepaid_subjects(t.state());
    assert!(
        subjects
            .is_active(t.state(), &booking.subject())
            .await
            .unwrap()
    );
    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("party_request"),
        staff(),
        now(),
    )
    .await
    .unwrap();
    assert!(
        !subjects
            .is_active(t.state(), &booking.subject())
            .await
            .unwrap()
    );
}
