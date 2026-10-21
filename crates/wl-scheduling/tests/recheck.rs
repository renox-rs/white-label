//! BL-SCH-08 (#175): the resource is asked again at check-in and start
//! (now), and `book`, `accept`, `reschedule` and `reassign` ask for the
//! booking's start (`SCH.ACL.04`), with the real `wl-hr`.
//!
//! The days are relative to today (the services take the real clock through
//! the routes): the licence of Dr A lapses on day 19, as on 20 November.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::documents::dubai_date;
use wl_kernel::workforce::EmployeeId;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal, Resource, ServiceType};

use support::{Booked, LICENCE, PATIENT, boot_hr, dubai, in_hours, now};

fn day(days: i64) -> NaiveDate {
    dubai_date(renox::db::now()) + TimeDelta::days(days)
}

/// `2026-11-20` as the text says it: `20 Nov 2026`.
fn said(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

fn path(id: i64, action: &str) -> String {
    format!("/scheduling/bookings/{id}/{action}")
}

/// Dr A and Dr B, both licensed for ever, and the service they deliver that
/// checks the party in and tracks the visit.
async fn two_doctors(t: &Booked) -> (Resource, Resource, ServiceType) {
    let a = t.practitioner("Dr A", 61, None).await;
    let b = t.practitioner("Dr B", 62, None).await;
    let service = t
        .service_with("consultation", &[&a, &b], |service| {
            service.required_qualifications = LICENCE.to_owned();
            service.check_in_required = true;
            service.track_in_progress = true;
        })
        .await;
    (a, b, service)
}

/// Dr A's licence is valid through `until` from now on.
fn licence_until(t: &Booked, doctor: &Resource, until: NaiveDate) {
    t.docs.accepted_until(
        &EmployeeId(doctor.employee_id.unwrap()).subject(),
        LICENCE,
        99,
        Some(until),
    );
}

/// Done when 1 and 2: Dr A's licence expired yesterday and the booking is
/// `confirmed` now: `check_in` is refused with the expiry and "reassign
/// first"; after the reassign to Dr B it succeeds.
#[renox::test]
async fn check_in_is_refused_until_the_booking_is_reassigned() {
    let t = boot_hr().await;
    let (a, b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&a], in_hours(1)).await;
    service::confirm(t.state(), booking.id, Actor::staff(1), now())
        .await
        .unwrap();
    licence_until(&t, &a, day(-1));

    let refused = service::check_in(t.state(), booking.id, Actor::staff(1), now()).await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the check-in is refused: {refused:?}");
    };
    assert_eq!(
        text,
        format!(
            "practitioner licence expired on {}; reassign first",
            said(day(-1))
        )
    );
    // Through the route: refused, nothing changed, nobody told.
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&path(booking.id, "check-in"), &[("x", "y")])
        .await
        .assert_status(422);
    let same = t.reload(booking.id).await;
    assert_eq!(same.status, BookingStatus::Confirmed);
    assert!(t.events.checked_in.all().is_empty());

    // Staff reassign to Dr B, who is licensed.
    t.app
        .htmx()
        .post(
            &path(booking.id, "reassign"),
            &[
                ("from", a.id.to_string().as_str()),
                ("to", b.id.to_string().as_str()),
                ("reason_code", "resource_ineligible"),
            ],
        )
        .await
        .assert_status(303);
    let moved = t.reload(booking.id).await;
    assert_eq!(moved.status, BookingStatus::Confirmed);
    assert_eq!(moved.resource_ids(t.db()).await.unwrap(), vec![b.id]);

    t.app
        .htmx()
        .post(&path(booking.id, "check-in"), &[("x", "y")])
        .await
        .assert_status(303);
    assert_eq!(
        t.reload(booking.id).await.status,
        BookingStatus::CheckedIn
    );
    assert_eq!(t.events.checked_in.all().len(), 1);
}

/// Done when 3: the licence lapses after the check-in; `start` is refused with
/// the hint to cancel with `resource_ineligible` and rebook, the booking stays
/// `checked_in`, a reassign is not possible any more, and the cancel the hint
/// names is a business cancellation.
#[renox::test]
async fn start_is_refused_after_the_check_in_and_the_hint_is_to_cancel_and_rebook() {
    let t = boot_hr().await;
    let (a, b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&a], in_hours(1)).await;
    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .expect("Dr A is licensed at the check-in");
    licence_until(&t, &a, day(-1));

    let refused = service::start(t.state(), booking.id, Actor::staff(1), now()).await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the start is refused: {refused:?}");
    };
    assert_eq!(
        text,
        format!(
            "practitioner licence expired on {}; cancel with resource_ineligible and rebook",
            said(day(-1))
        )
    );
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&path(booking.id, "start"), &[("x", "y")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload(booking.id).await.status,
        BookingStatus::CheckedIn
    );

    // Reassign is not available from `checked_in`.
    let reassigned = service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_ineligible",
        Actor::staff(1),
        now(),
    )
    .await;
    assert!(matches!(reassigned, Err(Refusal::Transition(_))));

    // The way out the hint names.
    t.app
        .htmx()
        .post(
            &path(booking.id, "cancel"),
            &[("reason_code", "resource_ineligible")],
        )
        .await
        .assert_status(303);
    let cancelled = t.reload(booking.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_code.as_deref(), Some("resource_ineligible"));
    assert_eq!(cancelled.cancel_initiated_by.as_deref(), Some("business"));
}

/// Done when 4, `book`: Dr A's licence ends on day 19; a booking on day 24 is
/// refused with the expiry named, on day 17 it is made.
#[renox::test]
async fn book_is_refused_after_the_expiry_and_made_before_it() {
    let t = boot_hr().await;
    let (a, _b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    licence_until(&t, &a, day(19));
    let party = PATIENT.0.to_string();
    let wanted = service.id.to_string();
    let doctor = a.id.to_string();
    let late = format!("{}T10:00", day(24));
    let early = format!("{}T10:00", day(17));

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            "/scheduling/bookings",
            &[
                ("party", party.as_str()),
                ("service", wanted.as_str()),
                ("resources", doctor.as_str()),
                ("starts_at", late.as_str()),
            ],
        )
        .await
        .assert_status(422);
    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[a.id], dubai(&day(24).to_string(), 10, 0)),
        Actor::staff(1),
        now(),
    )
    .await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the booking is refused: {refused:?}");
    };
    assert_eq!(
        text,
        format!("practitioner licence expired on {}", said(day(19)))
    );

    t.app
        .htmx()
        .post(
            "/scheduling/bookings",
            &[
                ("party", party.as_str()),
                ("service", wanted.as_str()),
                ("resources", doctor.as_str()),
                ("starts_at", early.as_str()),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(Booking::all(t.db()).await.unwrap().len(), 1);
}

/// The party who tries is told that the time cannot be booked, not why.
#[renox::test]
async fn a_party_is_not_told_why() {
    let t = boot_hr().await;
    let (a, _b, service) = two_doctors(&t).await;
    licence_until(&t, &a, day(19));

    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[a.id], dubai(&day(24).to_string(), 10, 0))
            .source("portal"),
        Actor::party(None),
        now(),
    )
    .await;
    let Err(Refusal::Invalid(text)) = refused else {
        panic!("the booking is refused: {refused:?}");
    };
    assert!(!text.contains("licence"), "{text}");
    assert!(!text.contains("expired"), "{text}");
}

/// Done when 4, `reschedule`: to day 24 refused, to day 18 moved.
#[renox::test]
async fn reschedule_is_refused_after_the_expiry() {
    let t = boot_hr().await;
    let (a, _b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&a], dubai(&day(10).to_string(), 10, 0)).await;
    licence_until(&t, &a, day(19));

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &path(booking.id, "reschedule"),
            &[("starts_at", format!("{}T10:00", day(24)).as_str())],
        )
        .await
        .assert_status(422);
    assert_eq!(
        t.reload(booking.id).await.starts_at,
        booking.starts_at,
        "not moved"
    );

    t.app
        .htmx()
        .post(
            &path(booking.id, "reschedule"),
            &[("starts_at", format!("{}T10:00", day(18)).as_str())],
        )
        .await
        .assert_status(303);
    assert_eq!(
        dubai_date(t.reload(booking.id).await.starts_at),
        day(18)
    );
}

/// Done when 4, `accept`: a request made while the licence was valid is
/// refused at acceptance when the licence ends before its start.
#[renox::test]
async fn accept_is_refused_after_the_expiry() {
    let t = boot_hr().await;
    let (a, _b, _service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let service = t
        .service_with("review", &[&a], |service| {
            service.requires_acceptance = true;
            service.required_qualifications = LICENCE.to_owned();
        })
        .await;
    let requested = service::request(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[a.id], dubai(&day(24).to_string(), 10, 0))
            .source("portal"),
        Actor::party(None),
        now(),
    )
    .await
    .expect("the licence is valid when the request is made");
    licence_until(&t, &a, day(19));

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&path(requested.id, "accept"), &[("x", "y")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload(requested.id).await.status,
        BookingStatus::Requested
    );
}

/// Done when 4, `reassign`: to a resource whose licence ends before the
/// start, refused; to one that holds it, done. The expiry is named to staff.
#[renox::test]
async fn reassign_to_a_lapsing_resource_is_refused() {
    let t = boot_hr().await;
    let (a, b, service) = two_doctors(&t).await;
    let booking = t
        .booking(&service, &[&a], dubai(&day(24).to_string(), 10, 0))
        .await;
    licence_until(&t, &b, day(19));

    let refused = service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_unavailable",
        Actor::staff(1),
        now(),
    )
    .await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the reassign is refused: {refused:?}");
    };
    assert!(text.contains(&said(day(19))), "{text}");

    licence_until(&t, &b, day(400));
    service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_unavailable",
        Actor::staff(1),
        now(),
    )
    .await
    .expect("Dr B holds the licence on that day");
    assert_eq!(
        t.reload(booking.id).await.resource_ids(t.db()).await.unwrap(),
        vec![b.id]
    );
}

/// A licence that is missing altogether is named as missing, and a service
/// that requires nothing is not refused.
#[renox::test]
async fn a_missing_licence_is_named_and_a_plain_service_needs_none() {
    let t = boot_hr().await;
    let employee = t.employee(70).await;
    let mut nobody = Resource::new("Dr N", wl_scheduling::ResourceType::Person)
        .employee(employee.id)
        .role("practitioner")
        .at(&support::branch());
    nobody.save(t.db()).await.unwrap();
    t.open_all_week(&nobody).await;
    let licensed = t.licensed_service(&[&nobody]).await;

    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, licensed.id, &[nobody.id], in_hours(30)),
        Actor::staff(1),
        now(),
    )
    .await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the booking is refused: {refused:?}");
    };
    assert_eq!(text, "practitioner licence is missing");

    let plain = t.service_with("walk_in", &[&nobody], |_| {}).await;
    service::book(
        t.state(),
        NewBooking::new(PATIENT, plain.id, &[nobody.id], in_hours(30)),
        Actor::staff(1),
        now(),
    )
    .await
    .expect("a service without requirements books anyone");
}
