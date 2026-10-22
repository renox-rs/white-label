//! The deposit refunds of #169, #170 and #171 asked of the real `wl-payments`
//! (and the party directory of the real `wl-parties`), not of a stand-in that
//! accepts everything: the refund row is there, `requested`, with the booking
//! as its source, and another user can approve it (`FIN.ACL.04`).

mod support;

use std::time::Duration;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::scheduling::WorkDone;
use wl_parties::Party;
use wl_payments::Refund;
use wl_payments::permissions as pay_perm;
use wl_payments::refund::RefundStatus;
use wl_scheduling::service::{self, Actor};
use wl_scheduling::{Booking, BookingStatus, links};

use support::{Booked, boot_real, in_hours};

const HOUR: Duration = Duration::from_secs(3600);

fn cancel_path(id: i64) -> String {
    format!("/scheduling/bookings/{id}/cancel")
}

/// A demo party of the real directory.
async fn patient(t: &Booked) -> PartyId {
    let party = Party::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a seeded party");
    PartyId(party.id)
}

/// The refunds `wl-payments` holds.
async fn refunds(t: &Booked) -> Vec<Refund> {
    Refund::all(t.db()).await.unwrap()
}

/// The one refund, checked against the booking it goes back to.
async fn the_refund(t: &Booked, booking: &Booking, amount: i64) -> Refund {
    let mut all = refunds(t).await;
    assert_eq!(all.len(), 1, "one refund is asked");
    let refund = all.remove(0);
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.source_kind, "booking");
    assert_eq!(refund.source_id, booking.id);
    assert_eq!(refund.amount, amount);
    assert_eq!(refund.currency, "AED");
    assert_eq!(refund.party_id, booking.party_id);
    assert_eq!(booking.deposit_refund_id, Some(refund.id));
    refund
}

/// Another user approves the refund through the route (`FIN.ACL.04`: the
/// refusal of the requester is `wl-payments`' own test).
async fn approved_by_a_second_user(t: &Booked, refund: &Refund) {
    let route = format!("/refunds/{}/transitions/approve", refund.id);
    let approver = t
        .user("Amal", &[pay_perm::VIEW, pay_perm::REFUNDS_APPROVE])
        .await;
    assert_ne!(refund.requested_by, Some(approver.id));
    t.app.acting_as(&approver);
    t.app
        .htmx()
        .post(&route, &[("x", "y")])
        .await
        .assert_status(303);
    let done = Refund::find_or_404(t.db(), refund.id).await.unwrap();
    assert_eq!(done.status, RefundStatus::Approved);
    assert_eq!(done.approved_by, Some(approver.id));
}

/// #169: the party cancels 30 hours before a booking with a 24 hour forfeit
/// window; the real module holds a `requested` refund of AED 200.
#[renox::test]
async fn an_early_party_cancel_gives_a_real_refund_row() {
    let t = boot_real().await;
    let party = patient(&t).await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let booking = t.book_for(party, &service, &[&doctor], in_hours(30)).await;

    let url = links::cancel_url(t.state(), booking.id, HOUR).unwrap();
    t.app
        .htmx()
        .post(&t.path_of(&url), &[("x", "y")])
        .await
        .assert_ok();

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    let refund = the_refund(&t, &done, 20_000).await;
    assert_eq!(refund.requested_by, None);
    approved_by_a_second_user(&t, &refund).await;
}

/// #169: staff cancel an hour before with a business reason; the whole
/// deposit is asked back, by the user who cancelled.
#[renox::test]
async fn a_business_cancel_gives_a_real_refund_in_full() {
    let t = boot_real().await;
    let party = patient(&t).await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let desk = t.front_desk().await;
    let booking = t.book_for(party, &service, &[&doctor], in_hours(1)).await;

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &cancel_path(booking.id),
            &[("reason_code", "equipment_failure")],
        )
        .await
        .assert_status(303);

    let done = t.reload(booking.id).await;
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    let refund = the_refund(&t, &done, 20_000).await;
    assert_eq!(refund.requested_by, Some(desk.id));
    approved_by_a_second_user(&t, &refund).await;
}

/// #169: a no-show forfeits (no refund), a second user waives it with a
/// reason, and the real module holds the refund the waiver asked for.
#[renox::test]
async fn a_waiver_gives_a_real_refund_row() {
    let t = boot_real().await;
    let party = patient(&t).await;
    let doctor = t.doctor("Dr A").await;
    let service = t.deposit_service(&doctor, true).await;
    let start = in_hours(1);
    let booking = t.book_for(party, &service, &[&doctor], start).await;

    service::mark_no_show(
        t.state(),
        booking.id,
        Actor::staff(1),
        start + TimeDelta::minutes(20),
    )
    .await
    .expect("the grace period is over");
    assert!(refunds(&t).await.is_empty());

    let waiver = t.waiver().await;
    t.app.acting_as(&waiver);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/waive-deposit", booking.id),
            &[("reason", "regular patient, was ill")],
        )
        .await
        .assert_status(303);

    let done = t.reload(booking.id).await;
    assert_eq!(done.deposit_outcome.as_deref(), Some("refunded"));
    let refund = the_refund(&t, &done, 20_000).await;
    assert_eq!(refund.requested_by, Some(waiver.id));
    approved_by_a_second_user(&t, &refund).await;
}

/// #170: AED 80 of work was done and the service type refunds the rest: the
/// real module holds a refund of AED 120.
#[renox::test]
async fn a_walk_out_gives_a_real_refund_of_what_is_left() {
    let t = boot_real().await;
    let party = patient(&t).await;
    let doctor = t.doctor("Dr A").await;
    let service = t.clinic_service(&doctor, false).await;
    let desk = t.front_desk().await;
    let booking = t.book_for(party, &service, &[&doctor], in_hours(1)).await;
    service::check_in(t.state(), booking.id, Actor::staff(desk.id), support::now())
        .await
        .expect("the party is checked in");
    t.work.set(booking.id, WorkDone::Captured { amount: 8_000 });

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(&cancel_path(booking.id), &[("reason_code", "walked_out")])
        .await
        .assert_status(303);

    let done = t.reload(booking.id).await;
    assert_eq!(done.deposit_applied, 8_000);
    assert_eq!(done.deposit_refunded, 12_000);
    let refund = the_refund(&t, &done, 12_000).await;
    assert_eq!(refund.requested_by, Some(desk.id));
    approved_by_a_second_user(&t, &refund).await;
}
