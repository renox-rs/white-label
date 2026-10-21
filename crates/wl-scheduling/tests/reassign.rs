//! Reassigning a booking to another resource at the same time (#171,
//! BL-SCH-04): the state and the time stay, the old slot is released, the
//! conflict clears, and the party is told who it is with now, without why.

mod support;

use renox::prelude::*;
use wl_kernel::scheduling::Initiator;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal, Resource, ServiceType};

use support::{Booked, Messages, PATIENT, boot, in_hours};

fn reassign_path(id: i64) -> String {
    format!("/scheduling/bookings/{id}/reassign")
}

/// Dr A, Dr B and a service they both deliver with a deposit of AED 200.
async fn two_doctors(t: &Booked) -> (Resource, Resource, ServiceType) {
    let a = t.doctor("Dr A").await;
    let b = t.doctor("Dr B").await;
    let service = t
        .service_with("consultation", &[&a, &b], |service| {
            service.deposit_amount = 20_000;
            service.deposit_forfeit_hours = 24;
            service.cancel_cutoff_hours = 24;
            service.check_in_required = true;
        })
        .await;
    (a, b, service)
}

/// Marks an open conflict on the booking, as a block would (BL-SCH-05 raises
/// it; this issue only clears it).
async fn conflict(t: &Booked, id: i64) {
    Booking::query()
        .where_(Booking::ID.eq(id))
        .update(t.db(), &[("conflict_block", &true)])
        .await
        .unwrap();
}

/// Booking K with Dr A has an open conflict; staff reassign it to Dr B with
/// `resource_unavailable`. K keeps its state, has Dr B, the conflict is
/// cleared, Dr A's slot is released and `BookingReassigned` is emitted.
#[renox::test]
async fn a_reassign_keeps_the_state_and_clears_the_conflict() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&a], in_hours(30)).await;
    conflict(&t, booking.id).await;

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &reassign_path(booking.id),
            &[
                ("from", a.id.to_string().as_str()),
                ("to", b.id.to_string().as_str()),
                ("reason_code", "resource_unavailable"),
            ],
        )
        .await
        .assert_status(303);

    let moved = t.reload(booking.id).await;
    assert_eq!(moved.status, BookingStatus::Booked);
    assert_eq!(moved.starts_at, booking.starts_at);
    assert_eq!(moved.ends_at, booking.ends_at);
    assert_eq!(moved.resource_ids(t.db()).await.unwrap(), [b.id]);
    assert!(!moved.has_conflict());
    assert_eq!(
        t.history(booking.id).await.last(),
        Some(&("reassign".to_owned(), "booked".to_owned()))
    );
    let events = t.events.reassigned.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].booking.0, booking.id);
    assert_eq!(events[0].old_resources, [a.id]);
    assert_eq!(events[0].new_resources, [b.id]);
    assert_eq!(events[0].reason_code, "resource_unavailable");
    let released = t.events.released.all();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].resources, [a.id]);
    assert_eq!(released[0].start, booking.starts_at);
    assert_eq!(released[0].end, booking.ends_at);

    // Dr A is free then: the same time can be booked with them again.
    t.booking(&service, &[&a], booking.starts_at).await;
    // And Dr B is not.
    let busy = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[b.id], booking.starts_at),
        Actor::staff(1),
        support::now(),
    )
    .await;
    assert!(matches!(busy, Err(Refusal::Taken { .. })), "{busy:?}");
}

/// The party's message names Dr B, has the three links and does not say why.
/// A party who cancels from it cancels as the business, 10 hours before the
/// start, and the AED 200 deposit is refunded in full.
#[renox::test]
async fn the_message_names_the_new_resource_and_cancelling_from_it_refunds() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let booking = t.booking(&service, &[&a], in_hours(10)).await;

    service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_unavailable",
        Actor::staff(1),
        support::now(),
    )
    .await
    .expect("Dr B is free and eligible");

    let sent = t.messages.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.type_key, "scheduling.booking_reassigned");
    assert_eq!(request.subject, booking.subject());
    assert_eq!(
        request.recipients,
        [wl_kernel::notify::NotifyRecipient::Party { id: PATIENT }]
    );
    let body = Messages::body(request);
    assert!(body.contains("Dr B"), "{body}");
    assert!(!body.contains("Dr A"), "{body}");
    assert!(body.contains(&booking.number), "{body}");
    let url = |key: &str| request.data[key].as_str().expect("a link").to_owned();
    assert!(url("accept_url").contains(&format!("/scheduling/links/{}/accept", booking.id)));
    assert!(
        url("reschedule_url").contains(&format!("/scheduling/links/{}/reschedule", booking.id))
    );
    assert!(url("cancel_url").contains(&format!(
        "/scheduling/links/{}/cancel/resource_unavailable",
        booking.id
    )));
    for link in ["accept_url", "reschedule_url", "cancel_url"] {
        assert!(body.contains(&url(link)), "{link} is in the text");
    }
    // The prose does not give the reason: take the links out of it.
    let prose: String = body
        .lines()
        .filter(|line| !line.contains("http"))
        .collect::<Vec<_>>()
        .join(" ");
    for word in ["unavailable", "illness", "sick", "reason"] {
        assert!(!prose.to_lowercase().contains(word), "{prose}");
    }

    // The party cancels from the message.
    t.app
        .htmx()
        .post(&t.path_of(&url("cancel_url")), &[("x", "y")])
        .await
        .assert_ok();
    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("resource_unavailable"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(t.refunds.amounts(), [20_000]);
    assert_eq!(
        t.events.cancelled.all()[0].initiated_by,
        Initiator::Business
    );
}

/// A resource that is busy then, or that is not eligible then, is refused;
/// nothing changes and nobody is told.
#[renox::test]
async fn a_busy_or_ineligible_resource_is_refused() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let c = t.doctor("Dr C").await;
    wl_scheduling::resource::offer(t.db(), service.id, c.id)
        .await
        .unwrap();
    let booking = t.booking(&service, &[&a], in_hours(30)).await;
    t.booking(&service, &[&b], booking.starts_at).await;
    t.licences.refuse(c.id, true);

    t.app.acting_as(&desk);
    for (to, expected) in [(b.id, 409), (c.id, 422)] {
        t.app
            .htmx()
            .post(
                &reassign_path(booking.id),
                &[
                    ("from", a.id.to_string().as_str()),
                    ("to", to.to_string().as_str()),
                    ("reason_code", "resource_unavailable"),
                ],
            )
            .await
            .assert_status(expected);
    }
    let same = t.reload(booking.id).await;
    assert_eq!(same.resource_ids(t.db()).await.unwrap(), [a.id]);
    assert!(t.events.reassigned.all().is_empty());
    assert!(t.events.released.all().is_empty());
    assert!(t.messages.sent().is_empty());
}

/// A reason is needed and must be one of the codes; only staff reassign.
#[renox::test]
async fn a_reassign_needs_a_known_reason_and_a_user_who_manages() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let booking = t.booking(&service, &[&a], in_hours(30)).await;
    let from = a.id.to_string();
    let to = b.id.to_string();

    t.app.acting_as(&t.front_desk().await);
    for reason in ["", "illness"] {
        t.app
            .htmx()
            .post(
                &reassign_path(booking.id),
                &[
                    ("from", from.as_str()),
                    ("to", to.as_str()),
                    ("reason_code", reason),
                ],
            )
            .await
            .assert_status(422);
    }
    t.app.acting_as(&t.viewer().await);
    t.app
        .htmx()
        .post(
            &reassign_path(booking.id),
            &[
                ("from", from.as_str()),
                ("to", to.as_str()),
                ("reason_code", "resource_unavailable"),
            ],
        )
        .await
        .assert_forbidden();
    let same = t.reload(booking.id).await;
    assert_eq!(same.resource_ids(t.db()).await.unwrap(), [a.id]);
    assert!(t.events.reassigned.all().is_empty());
}

/// A booking with a practitioner and a room: only the practitioner changes.
#[renox::test]
async fn only_the_replaced_resource_changes() {
    let t = boot().await;
    let a = t.doctor("Dr A").await;
    let b = t.doctor("Dr B").await;
    let room = t
        .resource("Room 1", wl_scheduling::ResourceType::Room)
        .await;
    t.open_all_week(&room).await;
    let service = t
        .service_with("procedure", &[&a, &b, &room], |service| {
            service.required_types = "person,room".to_owned();
        })
        .await;
    let booking = t.booking(&service, &[&a, &room], in_hours(30)).await;

    service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_unavailable",
        Actor::staff(1),
        support::now(),
    )
    .await
    .expect("Dr B takes over");
    let mut expected = vec![b.id, room.id];
    expected.sort_unstable();
    assert_eq!(
        t.reload(booking.id)
            .await
            .resource_ids(t.db())
            .await
            .unwrap(),
        expected
    );
    assert_eq!(t.events.reassigned.all()[0].old_resources.len(), 2);
    let released = t.events.released.all();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].resources, [a.id], "the room keeps its slot");
    // The room is still taken at that time.
    let again = service::book(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[a.id, room.id], booking.starts_at),
        Actor::staff(1),
        support::now(),
    )
    .await;
    assert!(matches!(again, Err(Refusal::Taken { .. })), "{again:?}");
}

/// A `confirmed` and a `requested` booking keep their state too.
#[renox::test]
async fn a_confirmed_or_requested_booking_keeps_its_state() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let confirmed = t.booking(&service, &[&a], in_hours(30)).await;
    service::confirm(t.state(), confirmed.id, Actor::staff(1), support::now())
        .await
        .unwrap();
    service::reassign(
        t.state(),
        confirmed.id,
        a.id,
        b.id,
        "site_closed",
        Actor::staff(1),
        support::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        t.reload(confirmed.id).await.status,
        BookingStatus::Confirmed
    );
    assert_eq!(
        t.history(confirmed.id).await.last(),
        Some(&("reassign_confirmed".to_owned(), "confirmed".to_owned()))
    );

    let c = t.doctor("Dr C").await;
    let d = t.doctor("Dr D").await;
    let asked = t
        .service_with("review", &[&c, &d], |service| {
            service.requires_acceptance = true;
        })
        .await;
    let requested = service::request(
        t.state(),
        NewBooking::new(PATIENT, asked.id, &[c.id], in_hours(30)),
        Actor::staff(1),
        support::now(),
    )
    .await
    .unwrap();
    service::reassign(
        t.state(),
        requested.id,
        c.id,
        d.id,
        "resource_unavailable",
        Actor::staff(1),
        support::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        t.reload(requested.id).await.status,
        BookingStatus::Requested
    );
    assert_eq!(
        t.reload(requested.id)
            .await
            .resource_ids(t.db())
            .await
            .unwrap(),
        [d.id]
    );
}

/// A booking that is checked in, or already over, is not reassigned.
#[renox::test]
async fn a_checked_in_booking_is_not_reassigned() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let booking = t.booking(&service, &[&a], in_hours(1)).await;
    service::check_in(t.state(), booking.id, Actor::staff(1), support::now())
        .await
        .unwrap();
    let refused = service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_unavailable",
        Actor::staff(1),
        support::now(),
    )
    .await;
    assert!(
        matches!(refused, Err(Refusal::Transition(_))),
        "{refused:?}"
    );
}

/// The check-in asks the vertical again whether the resource may deliver the
/// service now (`SCH.ACL.04`); when it may not the booking is reassigned
/// first, and then the party is checked in.
#[renox::test]
async fn check_in_asks_the_licence_again_and_reassign_is_the_way_out() {
    let t = boot().await;
    let (a, b, service) = two_doctors(&t).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&a], in_hours(1)).await;
    t.licences.refuse(a.id, true);

    t.app.acting_as(&desk);
    let check_in = format!("/scheduling/bookings/{}/check-in", booking.id);
    t.app
        .htmx()
        .post(&check_in, &[("x", "y")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);

    service::reassign(
        t.state(),
        booking.id,
        a.id,
        b.id,
        "resource_ineligible",
        Actor::staff(desk.id),
        support::now(),
    )
    .await
    .expect("Dr B holds a valid licence");
    t.app
        .htmx()
        .post(&check_in, &[("x", "y")])
        .await
        .assert_status(303);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::CheckedIn);
}
