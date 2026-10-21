//! The booking deposit follows who is at fault (#169, BL-SCH-02,
//! `SCH.ACL.05`): refunded when the business cancels or the party cancels
//! early, forfeited on a late party cancellation or a no-show, carried on a
//! reschedule, and waived by a user with `bookings.waive_deposit`.

mod support;

use std::time::Duration;

use renox::chrono::TimeDelta;
use wl_kernel::payments::RefundSource;
use wl_kernel::scheduling::{DepositOutcome, Initiator};
use wl_scheduling::pages::local_text;
use wl_scheduling::service::{self, Actor, CancelRequest};
use wl_scheduling::{BookingStatus, deposit, links};

use support::{PATIENT, boot, in_hours};

const HOUR: Duration = Duration::from_secs(3600);

fn cancel_path(id: i64) -> String {
    format!("/scheduling/bookings/{id}/cancel")
}

/// The party cancels 30 hours before a booking with a 24 hour forfeit window:
/// the whole deposit is asked back and the outcome is `refunded`.
#[renox::test]
async fn a_party_cancelling_outside_the_window_gets_the_deposit_back() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let booking = t.booking(&service, &[&doctor], in_hours(30)).await;
    assert_eq!(booking.deposit_amount, 20_000);

    let url = links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_ok();

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert!(!done.cancel_late);
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(done.deposit_refunded, 20_000);
    let asked = t.refunds.asked();
    assert_eq!(asked.len(), 1);
    let (request, by) = &asked[0];
    assert_eq!(request.amount, 20_000);
    assert_eq!(request.currency, "AED");
    assert_eq!(request.party, PATIENT);
    assert_eq!(request.source, RefundSource::Booking(booking.subject()));
    assert_eq!(*by, None);
    assert_eq!(done.deposit_refund_id, Some(901));
    assert_eq!(
        t.events.cancelled.all()[0].deposit_outcome,
        Some(DepositOutcome::Refunded)
    );
}

/// The party cancels 10 hours before: inside the window, so the deposit is
/// forfeited and nothing is asked back.
#[renox::test]
async fn a_party_cancelling_inside_the_window_forfeits_the_deposit() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let booking = t.booking(&service, &[&doctor], in_hours(10)).await;

    let url = links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_ok();

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert!(done.cancel_late);
    assert_eq!(done.deposit_outcome.as_deref(), Some("forfeited"));
    assert_eq!(done.deposit_forfeited, 20_000);
    assert!(t.refunds.asked().is_empty());
    assert_eq!(
        t.events.cancelled.all()[0].deposit_outcome,
        Some(DepositOutcome::Forfeited)
    );
}

/// Staff cancel with `equipment_failure` an hour before: the business is at
/// fault, so the whole deposit is asked back whatever the window.
#[renox::test]
async fn a_business_cancellation_refunds_the_deposit_in_full() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(1)).await;

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "equipment_failure")])
        .await
        .assert_status(303);

    let done = t.reload(booking.id).await;
    assert!(done.cancel_late);
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(t.refunds.amounts(), [20_000]);
    assert_eq!(t.refunds.asked()[0].1, Some(desk.id));
}

/// A party who cancels from a message about a change the business made
/// (code `resource_unavailable`, 10 hours before) is recorded as the
/// business and gets the deposit back in full.
#[renox::test]
async fn cancelling_from_a_change_message_refunds_the_deposit() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let booking = t.booking(&service, &[&doctor], in_hours(10)).await;

    let url = links::change_cancel_url(t.state(), booking.id, "resource_unavailable", HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_ok();

    let done = t.reload(booking.id).await;
    assert_eq!(done.cancel_code.as_deref(), Some("resource_unavailable"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(t.refunds.amounts(), [20_000]);
    let event = &t.events.cancelled.all()[0];
    assert_eq!(event.initiated_by, Initiator::Business);
    assert_eq!(event.deposit_outcome, Some(DepositOutcome::Refunded));
}

/// Staff move the booking: nothing is refunded or forfeited, the booking
/// keeps the deposit at its new time (`BookingRescheduled` says how much),
/// and a later cancellation by the business refunds it once.
#[renox::test]
async fn a_reschedule_carries_the_deposit_to_the_new_time() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(48)).await;
    let new_start = in_hours(72);

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/reschedule", booking.id),
            &[("starts_at", local_text(new_start).as_str())],
        )
        .await
        .assert_status(303);

    let moved = t.reload(booking.id).await;
    assert_eq!(moved.starts_at, new_start);
    assert_eq!(moved.deposit_amount, 20_000);
    assert_eq!(moved.deposit_outcome, None);
    assert!(t.refunds.asked().is_empty());
    assert_eq!(t.events.rescheduled.all()[0].deposit_carried, 20_000);

    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "site_closed")])
        .await
        .assert_status(303);
    assert_eq!(t.refunds.amounts(), [20_000]);
}

/// A no-show forfeits the deposit. A user with `bookings.waive_deposit`
/// waives it with a reason and a refund is asked; a user without the
/// permission is refused, and so is a waive without a reason or of a deposit
/// that is not forfeited.
#[renox::test]
async fn a_no_show_forfeits_and_a_waiver_turns_it_into_a_refund() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let start = in_hours(1);
    let booking = t.booking(&service, &[&doctor], start).await;

    service::mark_no_show(
        t.state(),
        booking.id,
        Actor::staff(1),
        start + TimeDelta::minutes(20),
    )
    .await
    .expect("the grace period is over");
    let forfeited = t.reload(booking.id).await;
    assert_eq!(forfeited.status, BookingStatus::NoShow);
    assert_eq!(forfeited.deposit_outcome.as_deref(), Some("forfeited"));
    assert_eq!(forfeited.deposit_forfeited, 20_000);
    assert!(t.refunds.asked().is_empty());
    assert!(t.events.no_show.all()[0].fee_applicable);

    let path = format!("/scheduling/bookings/{}/waive-deposit", booking.id);
    let desk = t.front_desk().await;
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&path, &[("reason", "regular patient, was ill")])
        .await
        .assert_forbidden();
    assert!(t.refunds.asked().is_empty());

    let waiver = t.waiver().await;
    t.app.acting_as(&waiver);
    t.app
        .htmx()
        .post(&path, &[("reason", "  ")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.deposit_forfeited, 20_000);

    t.app
        .htmx()
        .post(&path, &[("reason", "regular patient, was ill")])
        .await
        .assert_status(303);
    let waived = t.reload(booking.id).await;
    assert_eq!(waived.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(waived.deposit_forfeited, 0);
    assert_eq!(waived.deposit_refunded, 20_000);
    assert_eq!(waived.deposit_waived_by, Some(waiver.id));
    assert_eq!(
        waived.deposit_waive_reason.as_deref(),
        Some("regular patient, was ill")
    );
    assert!(waived.deposit_waived_at.is_some());
    assert_eq!(t.refunds.amounts(), [20_000]);
    assert_eq!(t.refunds.asked()[0].1, Some(waiver.id));

    // Waiving a refund makes no sense: nothing is asked twice.
    t.app
        .htmx()
        .post(&path, &[("reason", "again")])
        .await
        .assert_status(422);
    assert_eq!(t.refunds.amounts(), [20_000]);
}

/// A waive is offered on the booking page only to a user who may waive, and
/// only while a deposit is forfeited.
#[renox::test]
async fn the_waive_action_is_shown_to_those_who_may_use_it() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let booking = t.booking(&service, &[&doctor], in_hours(10)).await;
    let url = links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_ok();

    let page = format!("/scheduling/bookings/{}", booking.id);
    t.app.acting_as(&t.waiver().await);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("AED 200.00")
        .assert_see("forfeited")
        .assert_see("Waive forfeiture");
    t.app.acting_as(&t.front_desk().await);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_dont_see("Waive forfeiture");
}

/// A refund `wl-payments` refuses does not undo the cancellation: the
/// events go out and the booking shows no outcome until the settlement runs
/// again, which asks the refund once.
#[renox::test]
async fn a_refused_refund_leaves_the_cancellation_and_can_be_settled_again() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let desk = t.front_desk().await;
    let booking = t.booking(&service, &[&doctor], in_hours(1)).await;

    t.refunds.refuse(true);
    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "site_closed")])
        .await
        .assert_status(422);
    let cancelled = t.reload(booking.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.deposit_outcome, None);
    assert_eq!(cancelled.deposit_refunded, 0);
    assert_eq!(t.events.cancelled.all().len(), 1);
    assert_eq!(t.events.released.all().len(), 1);

    t.refunds.refuse(false);
    let settled = deposit::settle(t.state(), booking.id, Actor::staff(desk.id))
        .await
        .expect("the refund is accepted now");
    assert_eq!(settled.deposit_outcome.as_deref(), Some("refunded"));
    deposit::settle(t.state(), booking.id, Actor::staff(desk.id))
        .await
        .expect("a settled booking is left alone");
    assert_eq!(t.refunds.amounts(), [20_000]);
}

/// A service type without a deposit has nothing to settle.
#[renox::test]
async fn a_booking_without_a_deposit_settles_nothing() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let booking = t.booking(&service, &[&doctor], in_hours(1)).await;

    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("site_closed"),
        Actor::staff(1),
        support::now(),
    )
    .await
    .unwrap();
    let done = t.reload(booking.id).await;
    assert_eq!(done.deposit_outcome, None);
    assert!(t.refunds.asked().is_empty());
    assert_eq!(t.events.cancelled.all()[0].deposit_outcome, None);
}
