//! Cancelling a booking after the check-in (#170, BL-SCH-03, `SCH.ACL.06`):
//! only staff, only once the vertical's record has its work done captured or
//! marked none, and the deposit is applied to the work done for a party who
//! leaves.

mod support;

use std::time::Duration;

use renox::chrono::TimeDelta;
use wl_kernel::scheduling::{CancelStage, DepositOutcome, Initiator, WorkDone};
use wl_scheduling::service::{self, Actor, CancelRequest, NewBooking};
use wl_scheduling::{Booking, BookingStatus, Refusal, links};

use support::{Booked, in_hours};

const HOUR: Duration = Duration::from_secs(3600);

fn cancel_path(id: i64) -> String {
    format!("/scheduling/bookings/{id}/cancel")
}

/// A booking of the clinic service that is checked in (and `in_progress`
/// when `started`).
async fn checked_in(t: &Booked, walk_out_forfeits: bool, started: bool) -> Booking {
    let doctor = t.doctor("Dr A").await;
    let service = t.clinic_service(&doctor, walk_out_forfeits).await;
    let booking = t.booking(&service, &[&doctor], in_hours(1)).await;
    let at = support::now();
    service::check_in(t.state(), booking.id, Actor::staff(1), at)
        .await
        .expect("the party is checked in");
    if started {
        service::start(t.state(), booking.id, Actor::staff(1), at)
            .await
            .expect("the service starts");
    }
    t.reload(booking.id).await
}

/// While the visit's work done is `pending`, staff cannot cancel; once it is
/// marked none they can.
#[renox::test]
async fn a_cancel_is_refused_while_the_work_done_is_pending() {
    let t = boot_desk().await;
    let booking = checked_in(&t, false, false).await;
    assert_eq!(booking.status, BookingStatus::CheckedIn);
    t.work.set(booking.id, WorkDone::Pending);

    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::CheckedIn);
    assert!(t.events.cancelled.all().is_empty());
    assert!(t.refunds.asked().is_empty());

    t.work.set(booking.id, WorkDone::Nothing);
    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(303);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Cancelled);
}

/// Nothing was done, the party walked out and the service type refunds the
/// remainder: AED 200 is asked back, and the event says checked in, no work
/// done, refunded.
#[renox::test]
async fn a_walk_out_with_no_work_done_refunds_the_deposit_when_the_setting_refunds() {
    let t = boot_desk().await;
    let booking = checked_in(&t, false, false).await;
    t.work.set(booking.id, WorkDone::Nothing);

    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(t.refunds.amounts(), [20_000]);
    let event = &t.events.cancelled.all()[0];
    assert_eq!(event.stage, CancelStage::CheckedIn);
    assert_eq!(event.reason_code, "walked_out");
    assert_eq!(event.work_done_amount, 0);
    assert_eq!(event.deposit_applied, 0);
    assert_eq!(event.deposit_outcome, Some(DepositOutcome::Refunded));
}

/// AED 80 of work done and the setting forfeits the remainder: AED 80 is
/// applied to the work, AED 120 forfeited, nothing is refunded.
#[renox::test]
async fn a_walk_out_applies_the_deposit_to_the_work_and_forfeits_the_rest() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, true).await;
    assert_eq!(booking.status, BookingStatus::InProgress);
    t.work.set(booking.id, WorkDone::Captured { amount: 8_000 });

    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(
        done.deposit_outcome.as_deref(),
        Some("applied_to_work_done")
    );
    assert_eq!(done.deposit_applied, 8_000);
    assert_eq!(done.deposit_forfeited, 12_000);
    assert_eq!(done.deposit_refunded, 0);
    assert_eq!(done.work_done_amount, 8_000);
    assert!(t.refunds.asked().is_empty());
    let event = &t.events.cancelled.all()[0];
    assert_eq!(event.stage, CancelStage::InProgress);
    assert_eq!(event.work_done_amount, 8_000);
    assert_eq!(event.deposit_applied, 8_000);
    assert_eq!(
        event.deposit_outcome,
        Some(DepositOutcome::AppliedToWorkDone)
    );
}

/// With a setting that refunds, the rest after the work is paid back.
#[renox::test]
async fn a_walk_out_refunds_what_is_left_after_the_work_when_the_setting_refunds() {
    let t = boot_desk().await;
    let booking = checked_in(&t, false, false).await;
    t.work.set(booking.id, WorkDone::Captured { amount: 8_000 });

    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(done.deposit_applied, 8_000);
    assert_eq!(done.deposit_refunded, 12_000);
    assert_eq!(done.deposit_forfeited, 0);
    assert_eq!(t.refunds.amounts(), [12_000]);
}

/// A business cancel never applies the deposit: it is refunded in full
/// whether or not the party accepts the work done, which the vertical bills
/// on its own record (the event gives it the amount and an applied deposit of
/// nothing).
#[renox::test]
async fn a_business_cancel_refunds_the_deposit_in_full_and_applies_nothing() {
    let t = boot_desk().await;
    for accepted_work in [WorkDone::Nothing, WorkDone::Captured { amount: 8_000 }] {
        let booking = checked_in(&t, true, false).await;
        t.work.set(booking.id, accepted_work);
        t.app
            .htmx()
            .post(
                &cancel_path(booking.id),
                &[("reason_code", "equipment_failure")],
            )
            .await
            .assert_status(303);
        let done = t.reload(booking.id).await;
        assert_eq!(done.cancel_initiated_by.as_deref(), Some("business"));
        assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
        assert_eq!(done.deposit_applied, 0);
        let event = t.events.cancelled.all().pop().unwrap();
        assert_eq!(event.deposit_applied, 0);
        assert_eq!(event.deposit_outcome, Some(DepositOutcome::Refunded));
        if accepted_work != WorkDone::Nothing {
            assert_eq!(event.work_done_amount, 8_000);
        }
    }
    assert_eq!(t.refunds.amounts(), [20_000, 20_000]);
}

/// The party asks staff to stop: `party_request` after the check-in is
/// initiated by the party, so AED 80 is applied and AED 120 forfeited.
#[renox::test]
async fn a_party_who_asks_to_stop_is_treated_as_a_walk_out() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, false).await;
    t.work.set(booking.id, WorkDone::Captured { amount: 8_000 });

    t.app
        .htmx()
        .post(
            &cancel_path(booking.id),
            &[("reason_code", "party_request")],
        )
        .await
        .assert_status(303);
    let done = t.reload(booking.id).await;
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.deposit_applied, 8_000);
    assert_eq!(done.deposit_forfeited, 12_000);
    assert_eq!(t.events.cancelled.all()[0].initiated_by, Initiator::Party);
}

/// The party has no way to cancel once checked in: the link in the
/// confirmation is refused, and so is the portal.
#[renox::test]
async fn the_party_cannot_cancel_after_the_check_in() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, false).await;
    let url = links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    let path = t.path_of(&url);

    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_dont_see("Cancel this booking");
    t.app
        .htmx()
        .post(&path, &[("x", "y")])
        .await
        .assert_status(422);

    let change = links::change_cancel_url(t.state(), booking.id, "site_closed", HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&change), &[("x", "y")])
        .await
        .assert_status(422);

    let rita = t.user("Rita", &[]).await;
    t.portal.allow(rita.id, support::PATIENT);
    t.app.acting_as(&rita);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/portal/bookings/{}/cancel", booking.id),
            &[("x", "y")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::CheckedIn);
    assert!(t.events.cancelled.all().is_empty());
}

/// An `in_progress` booking cancelled 20 minutes into its 30 minutes frees
/// the last 10 minutes only.
#[renox::test]
async fn only_the_remaining_time_is_released() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, true).await;
    let at = booking.starts_at + TimeDelta::minutes(20);

    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("walked_out"),
        Actor::staff(1),
        at,
    )
    .await
    .expect("staff cancel after the check-in");
    let released = t.events.released.all();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].start, at);
    assert_eq!(released[0].end, booking.ends_at);
}

/// The time a walk-out released can be booked again: while the visit runs the
/// doctor is busy; after the party walked out 20 minutes in, a second party
/// books the same doctor from then on.
#[renox::test]
async fn the_released_time_can_be_booked_again() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, true).await;
    let doctor = booking.resource_ids(t.db()).await.unwrap()[0];
    let service = booking.service_type_id;
    let at = booking.starts_at + TimeDelta::minutes(20);

    // While the visit runs, the doctor is busy.
    let busy = service::book(
        t.state(),
        NewBooking::new(
            support::OTHER_PATIENT,
            service,
            &[doctor],
            booking.starts_at,
        ),
        Actor::staff(1),
        support::now(),
    )
    .await;
    assert!(matches!(busy, Err(Refusal::Taken { .. })), "{busy:?}");

    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("walked_out"),
        Actor::staff(1),
        at,
    )
    .await
    .expect("staff cancel after the check-in");
    assert_eq!(t.events.released.all()[0].start, at);

    let again = service::book(
        t.state(),
        NewBooking::new(support::OTHER_PATIENT, service, &[doctor], at),
        Actor::staff(1),
        support::now(),
    )
    .await
    .expect("the released time is bookable again");
    assert_eq!(again.status, BookingStatus::Booked);
    assert_eq!(again.starts_at, at);
}

/// After the end nothing is released.
#[renox::test]
async fn nothing_is_released_after_the_end() {
    let t = boot_desk().await;
    let booking = checked_in(&t, true, false).await;
    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("walked_out"),
        Actor::staff(1),
        booking.ends_at + TimeDelta::minutes(5),
    )
    .await
    .expect("staff cancel after the check-in");
    assert!(t.events.released.all().is_empty());
}

/// A booked, unchecked booking is cancelled by the old rules, and
/// `walked_out` is still refused before the check-in.
#[renox::test]
async fn walked_out_is_for_a_party_who_was_checked_in() {
    let t = boot_desk().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.clinic_service(&doctor, true).await;
    let booking = t.booking(&service, &[&doctor], in_hours(5)).await;
    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(422);
}

/// The desk's pages need a user who manages bookings.
async fn boot_desk() -> Booked {
    let t = support::boot().await;
    let desk = t.front_desk().await;
    t.app.acting_as(&desk);
    t
}
