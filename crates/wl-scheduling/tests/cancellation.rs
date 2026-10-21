//! Cancellation reason codes and who initiated (#168, BL-SCH-01): every
//! cancellation records a code and a side, `BookingCancelled` carries them,
//! and *late* follows the forfeit window or the cut-off.

mod support;

use std::time::Duration;

use renox::chrono::TimeDelta;
use wl_kernel::scheduling::{CancelStage, CancelledBy, Initiator};
use wl_scheduling::service::{self, Actor, CancelRequest};
use wl_scheduling::{BookingStatus, WlScheduling};

use support::{PATIENT, boot, boot_with, in_hours, now};

const HOUR: Duration = Duration::from_secs(3600);

/// Staff cancel a `booked` booking with `site_closed`: the business caused
/// it, and the event says so.
#[renox::test]
async fn staff_cancel_with_a_business_code() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", booking.id),
            &[("reason_code", "site_closed")],
        )
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("site_closed"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(done.cancel_stage.as_deref(), Some("before_start"));
    let events = t.events.cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].booking.0, booking.id);
    assert_eq!(events[0].reason_code, "site_closed");
    assert_eq!(events[0].initiated_by, Initiator::Business);
    assert_eq!(events[0].stage, CancelStage::BeforeStart);
    assert_eq!(events[0].by, CancelledBy::Staff);
    assert_eq!(events[0].deposit_outcome, None);
}

/// Staff record the other side only with a note.
#[renox::test]
async fn the_side_can_be_overridden_only_with_a_note() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let path = format!("/scheduling/bookings/{}/cancel", booking.id);

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &path,
            &[("reason_code", "site_closed"), ("initiated_by", "party")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);

    t.app
        .htmx()
        .post(
            &path,
            &[
                ("reason_code", "site_closed"),
                ("initiated_by", "party"),
                ("note", "patient asked to close early"),
            ],
        )
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.cancel_note.as_deref(), Some("patient asked to close early"));
    assert_eq!(t.events.cancelled.all()[0].initiated_by, Initiator::Party);
}

/// A cancellation without a code, or with one the module does not know, is
/// refused.
#[renox::test]
async fn a_cancellation_needs_a_known_reason_code() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let path = format!("/scheduling/bookings/{}/cancel", booking.id);
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&path, &[("reason_code", "")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&path, &[("reason_code", "bad_weather")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);
    assert!(t.events.cancelled.all().is_empty());
}

/// A party cancels from the link in a reminder: `party_request`, initiated
/// by the party, and only before the cut-off.
#[renox::test]
async fn a_party_cancels_from_the_reminder_link() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let url = wl_scheduling::links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    let path = t.path_of(&url);

    t.app.get(&path).await.assert_ok();
    t.app.htmx().post(&path, &[("x", "y")]).await.assert_ok();
    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("party_request"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.cancel_by.as_deref(), Some("party"));
    let events = t.events.cancelled.all();
    assert_eq!(events[0].by, CancelledBy::Party);
    assert_eq!(events[0].initiated_by, Initiator::Party);

    // A link whose signature is altered does nothing.
    let other = t.booking(&service, &[&doctor], in_hours(72)).await;
    let url = wl_scheduling::links::cancel_url(t.state(), other.id, HOUR).unwrap();
    let forged = t.path_of(&url).replace("signature=", "signature=0");
    t.app
        .htmx()
        .post(&forged, &[("x", "y")])
        .await
        .assert_forbidden();
    assert_eq!(t.reload(other.id).await.status, BookingStatus::Booked);
}

/// The ordinary link stops working at the cancellation cut-off; the link of
/// a change message does not, and records the business and the code of the
/// change.
#[renox::test]
async fn a_change_message_link_cancels_as_the_business_after_the_cut_off() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(3)).await;

    let ordinary = wl_scheduling::links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&ordinary), &[("x", "y")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);

    let change = wl_scheduling::links::change_cancel_url(
        t.state(),
        booking.id,
        "resource_unavailable",
        HOUR,
    )
    .unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&change), &[("x", "y")])
        .await
        .assert_ok();
    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("resource_unavailable"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(done.cancel_by.as_deref(), Some("party"));
    let events = t.events.cancelled.all();
    assert_eq!(events[0].reason_code, "resource_unavailable");
    assert_eq!(events[0].initiated_by, Initiator::Business);
}

/// A link for a code the module does not know is a 404; the party cannot make
/// one up.
#[renox::test]
async fn a_change_link_with_an_unknown_code_is_refused() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(3)).await;
    let url =
        wl_scheduling::links::change_cancel_url(t.state(), booking.id, "made_up", HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_not_found();
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);
}

/// A portal user cancels only the bookings of the party they act for.
#[renox::test]
async fn a_portal_user_cancels_their_own_booking() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let mine = t.booking(&service, &[&doctor], in_hours(48)).await;
    let theirs = t
        .book_for(support::OTHER_PATIENT, &service, &[&doctor], in_hours(72))
        .await;
    let sara = t.user("Sara", &[]).await;
    t.portal.allow(sara.id, PATIENT);

    t.app.acting_as(&sara);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/portal/bookings/{}/cancel", theirs.id),
            &[("x", "y")],
        )
        .await
        .assert_not_found();
    t.app
        .htmx()
        .post(
            &format!("/scheduling/portal/bookings/{}/cancel", mine.id),
            &[("x", "y")],
        )
        .await
        .assert_ok();
    let done = t.reload(mine.id).await;
    assert_eq!(done.cancel_code.as_deref(), Some("party_request"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(t.reload(theirs.id).await.status, BookingStatus::Booked);
}

/// With a deposit, a cancellation is late inside the forfeit window (12 h)
/// whatever the cut-off (24 h); without one it is late inside the cut-off.
#[renox::test]
async fn late_follows_the_forfeit_window_or_the_cut_off() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let deposit = t
        .service_with("deposit", &[&doctor], |s| {
            s.deposit_amount = 20_000;
            s.deposit_forfeit_hours = 12;
            s.cancel_cutoff_hours = 24;
        })
        .await;
    let plain = t
        .service_with("plain", &[&doctor], |s| s.cancel_cutoff_hours = 24)
        .await;
    let (first, second, third) = (in_hours(48), in_hours(50), in_hours(52));
    let a = t.booking(&deposit, &[&doctor], first).await;
    let b = t.booking(&deposit, &[&doctor], second).await;
    let c = t.booking(&plain, &[&doctor], third).await;
    let by_party = || CancelRequest::new("party_request");

    let late = service::cancel(
        t.state(),
        a.id,
        by_party(),
        Actor::staff(1),
        first - TimeDelta::hours(10),
    )
    .await
    .unwrap();
    assert!(late.cancel_late);
    let early = service::cancel(
        t.state(),
        b.id,
        by_party(),
        Actor::staff(1),
        second - TimeDelta::hours(20),
    )
    .await
    .unwrap();
    assert!(!early.cancel_late);
    let no_deposit = service::cancel(
        t.state(),
        c.id,
        by_party(),
        Actor::staff(1),
        third - TimeDelta::hours(20),
    )
    .await
    .unwrap();
    assert!(no_deposit.cancel_late);
    let lates: Vec<bool> = t.events.cancelled.all().iter().map(|e| e.late).collect();
    assert_eq!(lates, [true, false, true]);
}

/// After check-in only staff cancel, and `walked_out` is for after check-in
/// only.
#[renox::test]
async fn walking_out_is_for_after_check_in_and_for_staff() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("visit", &[&doctor], |s| {
            s.check_in_required = true;
            s.track_in_progress = true;
        })
        .await;
    let booking = t.booking(&service, &[&doctor], in_hours(3)).await;
    let walked = || CancelRequest::new("walked_out");

    // Before check-in `walked_out` is refused.
    let early = service::cancel(t.state(), booking.id, walked(), Actor::staff(1), now()).await;
    assert!(early.is_err());

    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .unwrap();
    // The party cannot cancel now, even from a change message.
    let party = service::cancel(
        t.state(),
        booking.id,
        CancelRequest::from_change_message("resource_unavailable"),
        Actor::party(None),
        now(),
    )
    .await;
    assert!(party.is_err());
    let ordinary = service::cancel(
        t.state(),
        booking.id,
        CancelRequest::by_party(),
        Actor::party(None),
        now(),
    )
    .await;
    assert!(ordinary.is_err());
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::CheckedIn);

    let done = service::cancel(t.state(), booking.id, walked(), Actor::staff(1), now())
        .await
        .unwrap();
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.cancel_stage.as_deref(), Some("checked_in"));
    assert_eq!(t.events.cancelled.all()[0].stage, CancelStage::CheckedIn);
}

/// `decline` records `other` with the staff's reason, initiated by the
/// business, and sends `BookingCancelled`.
#[renox::test]
async fn a_decline_records_other_and_the_business() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let requested = service::request(
        t.state(),
        service::NewBooking::new(PATIENT, service.id, &[doctor.id], in_hours(48)),
        Actor::party(None),
        now(),
    )
    .await
    .unwrap();
    let declined = service::decline(
        t.state(),
        requested.id,
        "Doctor is on leave",
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(declined.cancel_code.as_deref(), Some("other"));
    assert_eq!(declined.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(declined.cancel_note.as_deref(), Some("Doctor is on leave"));
    let events = t.events.cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason_code, "other");
    assert_eq!(events[0].initiated_by, Initiator::Business);
}

/// A vertical adds a code with its side; the module's codes keep theirs.
#[renox::test]
async fn a_vertical_adds_a_reason_code() {
    let module = WlScheduling::new().reason_code("patient_unwell", Initiator::Party);
    let t = boot_with(module).await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let done = service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("patient_unwell"),
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(done.cancel_code.as_deref(), Some("patient_unwell"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
}
