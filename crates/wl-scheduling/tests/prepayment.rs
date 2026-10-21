//! BL-SCH-10 (#177): the prepayment hold of a self-booking, with the real
//! `wl-payments` and its fake gateway: the payment link expires when the hold
//! does, `PaymentLinkExpired` and `SCH.RUL.04` cancel the unpaid booking once,
//! the slot goes to the waiting list, `PaymentFailed` keeps the hold, and a
//! payment that arrives after the release is refunded in full once.

mod support;

use std::time::Duration;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::payments::{PaymentFailureKind, PaymentId, PaymentMethod, PaymentReceived};
use wl_kernel::scheduling::Initiator;
use wl_parties::Party;
use wl_payments::Refund;
use wl_payments::online;
use wl_payments::payment::Payment;
use wl_payments::payment_link::{self, LinkStatus, PaymentLink};
use wl_payments::refund::RefundStatus;
use wl_scheduling::prepayment::{self, PrepaymentRefund};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::waiting::{self, NewEntry, WaitingStatus};
use wl_scheduling::{
    Booking, BookingStatus, Refusal, Resource, ServiceType, WaitingEntry, WlScheduling, rules,
};

use support::{Booked, boot_paid, boot_paid_with, boot_real, in_hours, now};

/// A demo party of the real directory.
async fn patient(t: &Booked) -> PartyId {
    let party = Party::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a seeded party");
    PartyId(party.id)
}

/// Dr A and a service with a prepayment of AED 150.
async fn clinic(t: &Booked, acceptance: bool) -> (Resource, ServiceType) {
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("consultation", &[&doctor], |service| {
            service.prepayment_amount = 15_000;
            service.requires_acceptance = acceptance;
        })
        .await;
    (doctor, service)
}

/// The party books by themselves (through the portal's service call) at `at`.
async fn self_book(
    t: &Booked,
    party: PartyId,
    service: &ServiceType,
    doctor: &Resource,
    start: renox::db::DateTime,
    at: renox::db::DateTime,
) -> std::result::Result<Booking, Refusal> {
    let new = NewBooking::new(party, service.id, &[doctor.id], start).source("portal");
    if service.requires_acceptance {
        service::request(t.state(), new, Actor::party(None), at).await
    } else {
        service::book(t.state(), new, Actor::party(None), at).await
    }
}

/// The payment link of `booking`.
async fn link_of(t: &Booked, booking: &Booking) -> PaymentLink {
    let id = t.reload(booking.id).await.prepayment_id.expect("a link");
    PaymentLink::find_or_404(t.db(), id).await.unwrap()
}

/// The party starts paying on the link; the gateway has the transaction.
async fn start_paying(t: &Booked, link: &PaymentLink) -> Payment {
    online::start_online(t.state(), link.id)
        .await
        .expect("the payment starts")
        .payment
}

fn txn(payment: &Payment) -> String {
    payment.gateway_transaction.clone().expect("a transaction")
}

async fn refunds(t: &Booked) -> Vec<Refund> {
    Refund::all(t.db()).await.unwrap()
}

fn told(t: &Booked, type_key: &str) -> usize {
    t.messages
        .sent()
        .iter()
        .filter(|request| request.type_key == type_key)
        .count()
}

/// Done when 1, first half: the link is made for the prepayment, for the
/// booking, and expires when the 15 minute hold does; a booking made by staff
/// has none.
#[renox::test]
async fn a_self_booking_asks_a_link_that_expires_with_its_hold() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let at = now();

    let booking = self_book(&t, party, &service, &doctor, in_hours(30), at)
        .await
        .expect("the booking is made");

    assert_eq!(booking.status, BookingStatus::Booked);
    assert!(booking.payment_pending);
    assert_eq!(booking.hold_until, Some(at + TimeDelta::minutes(15)));
    let link = link_of(&t, &booking).await;
    assert_eq!(link.amount, 15_000);
    assert_eq!(link.currency, "AED");
    assert_eq!(link.expires_at, at + TimeDelta::minutes(15));
    assert_eq!(link.prepaid(), Some(booking.subject()));
    assert_eq!(link.party(), party);
    assert_eq!(link.status, LinkStatus::Open);
    let path = booking.prepayment_path.clone().expect("the page to pay on");
    assert!(path.starts_with("/pay/"), "{path}");

    // Staff book without a payment.
    let other = in_hours(60);
    let by_staff = t.book_for(party, &service, &[&doctor], other).await;
    assert!(!by_staff.payment_pending);
    assert_eq!(by_staff.prepayment_id, None);
    assert_eq!(by_staff.hold_until, None);
}

/// Done when 1 (second half) and 2: `PaymentLinkExpired` cancels the booking
/// as `unpaid`, initiated by the party, with no deposit outcome; the slot is
/// released, offered to the first waiting entry, and the party is told.
#[renox::test]
async fn the_link_expiry_cancels_the_unpaid_booking_and_offers_the_slot() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let waiting_party = PartyId(party.0 + 1000);
    let entry = waiting::join(t.state(), NewEntry::new(waiting_party, service.id))
        .await
        .unwrap();
    let start = in_hours(30);
    let booking = self_book(&t, party, &service, &doctor, start, now())
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;

    // The hourly job of wl-payments, at the moment the link expires.
    let expired = payment_link::expire_due(t.state(), link.expires_at)
        .await
        .unwrap();
    assert_eq!(expired, 1);

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("unpaid"));
    assert_eq!(done.cancel_initiated_by.as_deref(), Some("party"));
    assert_eq!(done.cancel_by.as_deref(), Some("system"));
    assert_eq!(done.deposit_outcome, None);
    assert!(refunds(&t).await.is_empty(), "nothing to refund or forfeit");
    let cancelled = t.events.cancelled.all();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].reason_code, "unpaid");
    assert_eq!(cancelled[0].initiated_by, Initiator::Party);
    assert_eq!(cancelled[0].deposit_outcome, None);
    assert_eq!(t.events.released.all().len(), 1);
    assert_eq!(told(&t, "scheduling.booking_unpaid"), 1);
    // Done when 2: the first matching entry is offered the slot.
    let offered = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(offered.status, WaitingStatus::Offered);
    assert_eq!(offered.offered_from, Some(start));
    assert_eq!(offered.offered_resource_id, Some(doctor.id));
}

/// Done when 3 and 6: a payment confirmed after the release is refunded in
/// full once with the reason *slot not available*, though the slot is still
/// free; the booking stays `cancelled`, and a replayed `PaymentReceived` asks
/// nothing more.
#[renox::test]
async fn a_late_payment_is_refunded_in_full_once_and_the_booking_stays_cancelled() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let start = in_hours(30);
    let booking = self_book(&t, party, &service, &doctor, start, now())
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;
    // The party was on the gateway's page when the hold ended.
    let paying = start_paying(&t, &link).await;
    payment_link::expire_due(t.state(), link.expires_at)
        .await
        .unwrap();
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Cancelled);

    let paid = online::gateway_succeeded(t.state(), &txn(&paying), 15_000)
        .await
        .unwrap()
        .expect("the payment is confirmed");

    let all = refunds(&t).await;
    assert_eq!(all.len(), 1, "one refund is asked");
    assert_eq!(all[0].status, RefundStatus::Requested);
    assert_eq!(all[0].source_kind, "payment");
    assert_eq!(all[0].source_id, paid.id);
    assert_eq!(all[0].amount, 15_000);
    assert_eq!(all[0].currency, "AED");
    assert_eq!(all[0].party_id, party.0);
    assert_eq!(all[0].reason, "slot not available");
    let stays = t.reload(booking.id).await;
    assert_eq!(stays.status, BookingStatus::Cancelled, "not revived");
    assert_eq!(stays.cancel_code.as_deref(), Some("unpaid"));
    assert_eq!(told(&t, "scheduling.late_payment_refunded"), 1);
    let claims = PrepaymentRefund::all(t.db()).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].refund_id, Some(all[0].id));

    // The same payment received again: nothing more.
    t.state()
        .emit(PaymentReceived {
            payment: PaymentId(paid.id),
            party,
            amount: 15_000,
            method: PaymentMethod::Gateway,
            allocations: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(refunds(&t).await.len(), 1);
    assert_eq!(PrepaymentRefund::all(t.db()).await.unwrap().len(), 1);
    assert_eq!(told(&t, "scheduling.late_payment_refunded"), 1);

    // The slot is free: it can be booked again, as a new booking.
    let again = t.book_for(party, &service, &[&doctor], start).await;
    assert_ne!(again.id, booking.id);
}

/// The refund of a late payment is in the payment's own currency, not the
/// module's default.
#[renox::test]
async fn the_late_refund_is_in_the_currency_of_the_payment() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), now())
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;
    let paying = start_paying(&t, &link).await;
    payment_link::expire_due(t.state(), link.expires_at)
        .await
        .unwrap();
    Payment::query()
        .where_(Payment::ID.eq(paying.id))
        .update(t.db(), &[("currency", &"USD".to_owned())])
        .await
        .unwrap();

    let paid = online::gateway_succeeded(t.state(), &txn(&paying), 15_000)
        .await
        .unwrap()
        .expect("the payment is confirmed");

    let all = refunds(&t).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].source_id, paid.id);
    assert_eq!(all[0].currency, "USD");
}

/// Done when 4: a failed payment keeps the hold; the retry on the same link
/// that succeeds keeps the booking and asks no refund; the link that expires
/// afterwards, with the booking paid, is ignored.
#[renox::test]
async fn a_failed_payment_keeps_the_hold_and_a_retry_keeps_the_booking() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), now())
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;
    let held = t.reload(booking.id).await.hold_until;

    // Minute 5: the card is declined.
    let first = start_paying(&t, &link).await;
    online::gateway_failed(
        t.state(),
        &txn(&first),
        PaymentFailureKind::Declined,
        "Card declined",
    )
    .await
    .unwrap()
    .expect("the payment failed");
    let waiting = t.reload(booking.id).await;
    assert_eq!(waiting.status, BookingStatus::Booked);
    assert!(waiting.payment_pending, "still unpaid, still held");
    assert_eq!(waiting.hold_until, held);
    assert_eq!(
        PaymentLink::find_or_404(t.db(), link.id)
            .await
            .unwrap()
            .status,
        LinkStatus::Open,
        "the party may retry on the same link"
    );
    assert!(t.events.cancelled.all().is_empty());

    // Minute 8: the retry is paid.
    let second = start_paying(&t, &link).await;
    online::gateway_succeeded(t.state(), &txn(&second), 15_000)
        .await
        .unwrap()
        .expect("the payment is confirmed");
    let kept = t.reload(booking.id).await;
    assert_eq!(kept.status, BookingStatus::Booked);
    assert!(!kept.payment_pending);
    assert_eq!(kept.hold_until, None);
    assert!(refunds(&t).await.is_empty(), "no refund is asked");

    // The link's expiry afterwards finds the booking paid.
    let still = PaymentLink::find_or_404(t.db(), link.id).await.unwrap();
    payment_link::expire_due(t.state(), still.expires_at)
        .await
        .unwrap();
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Booked);
    assert!(t.events.cancelled.all().is_empty());
    assert_eq!(told(&t, "scheduling.booking_unpaid"), 0);
}

/// Done when 5, the link first: a `requested` self-booking with no payment is
/// cancelled once at minute 15, whichever trigger comes first. The request
/// hold (5 minutes) did not shorten the hold.
#[renox::test]
async fn a_requested_unpaid_booking_is_cancelled_once_when_the_link_expires_first() {
    let t = boot_paid_with(WlScheduling::new().request_hold_minutes(5)).await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, true).await;
    let at = now();
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), at)
        .await
        .unwrap();
    assert_eq!(booking.status, BookingStatus::Requested);
    assert_eq!(booking.hold_until, Some(at + TimeDelta::minutes(15)));
    let link = link_of(&t, &booking).await;
    let minute_15 = at + TimeDelta::minutes(15);

    payment_link::expire_due(t.state(), minute_15)
        .await
        .unwrap();
    let swept = rules::run(t.state(), minute_15).await.unwrap();

    assert_eq!(swept.released, 0, "the second trigger finds it cancelled");
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Cancelled);
    assert_eq!(t.events.cancelled.all().len(), 1);
    assert_eq!(t.events.released.all().len(), 1);
    assert_eq!(told(&t, "scheduling.booking_unpaid"), 1);
    assert_eq!(
        PaymentLink::find_or_404(t.db(), link.id)
            .await
            .unwrap()
            .status,
        LinkStatus::Expired
    );
}

/// Done when 5, the rule first (`SCH.RUL.04`, in the five-minute run): the
/// hold is not over a minute before, and the link's expiry afterwards does
/// nothing more.
#[renox::test]
async fn a_requested_unpaid_booking_is_cancelled_once_when_the_rule_runs_first() {
    let t = boot_paid_with(WlScheduling::new().request_hold_minutes(5)).await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, true).await;
    let at = now();
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), at)
        .await
        .unwrap();
    let minute_15 = at + TimeDelta::minutes(15);

    let early = rules::run(t.state(), minute_15 - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(early.released, 0, "the hold is not over");
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Requested);

    let swept = rules::run(t.state(), minute_15).await.unwrap();
    assert_eq!(swept.released, 1);
    payment_link::expire_due(t.state(), minute_15)
        .await
        .unwrap();

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("unpaid"));
    assert_eq!(t.events.cancelled.all().len(), 1, "one BookingCancelled");
    assert_eq!(told(&t, "scheduling.booking_unpaid"), 1);
    // A later run of the rule has nothing left to release.
    assert_eq!(
        prepayment::release_due(t.state(), minute_15).await.unwrap(),
        0
    );
}

/// `SCH.RUL.04` (#177): the rule that releases an unpaid hold also closes its
/// payment link, so the party cannot start paying for a slot that is gone; the
/// link's own expiry afterwards finds nothing to do, and a release by the
/// link's expiry leaves the (already expired) link as it is.
#[renox::test]
async fn the_rule_that_releases_a_hold_closes_its_payment_link() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let at = now();
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), at)
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;
    assert_eq!(link.status, LinkStatus::Open);

    let swept = rules::run(t.state(), link.expires_at).await.unwrap();
    assert_eq!(swept.released, 1);
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Cancelled);
    let closed = PaymentLink::find_or_404(t.db(), link.id).await.unwrap();
    assert_eq!(closed.status, LinkStatus::Cancelled);
    assert!(
        online::start_online(t.state(), link.id).await.is_err(),
        "the party can no longer pay on it"
    );
    assert_eq!(
        payment_link::expire_due(t.state(), link.expires_at)
            .await
            .unwrap(),
        0,
        "a closed link does not expire"
    );
    assert_eq!(t.events.cancelled.all().len(), 1, "one BookingCancelled");

    // The other way round: the link's expiry cancels the booking and the
    // link stays `expired`.
    let other = self_book(&t, party, &service, &doctor, in_hours(40), at)
        .await
        .unwrap();
    let second = link_of(&t, &other).await;
    payment_link::expire_due(t.state(), second.expires_at)
        .await
        .unwrap();
    assert_eq!(t.reload(other.id).await.status, BookingStatus::Cancelled);
    let expired = PaymentLink::find_or_404(t.db(), second.id).await.unwrap();
    assert_eq!(expired.status, LinkStatus::Expired);
}

/// Staff who accept an unpaid request keep the unpaid hold: the booking is
/// `booked`, still awaiting its payment, and released when the link expires.
#[renox::test]
async fn accepting_an_unpaid_request_keeps_the_hold() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, true).await;
    let at = now();
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), at)
        .await
        .unwrap();

    let accepted = service::accept(t.state(), booking.id, Actor::staff(1), at)
        .await
        .expect("accepted");
    assert_eq!(accepted.status, BookingStatus::Booked);
    assert!(accepted.payment_pending);
    assert_eq!(accepted.hold_until, Some(at + TimeDelta::minutes(15)));

    let link = link_of(&t, &booking).await;
    payment_link::expire_due(t.state(), link.expires_at)
        .await
        .unwrap();
    assert_eq!(t.reload(booking.id).await.status, BookingStatus::Cancelled);
}

/// A payment that comes after the hold but before anything released the
/// booking: it is released first and the money goes back.
#[renox::test]
async fn a_payment_after_the_hold_but_before_the_release_is_refunded() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let booking = self_book(&t, party, &service, &doctor, in_hours(30), now())
        .await
        .unwrap();
    let link = link_of(&t, &booking).await;
    let paying = start_paying(&t, &link).await;

    // Sixteen minutes later the gateway confirms; no job ran in between.
    t.app.travel(Duration::from_secs(16 * 60));
    t.app
        .at_travelled_time(async {
            online::gateway_succeeded(t.state(), &txn(&paying), 15_000)
                .await
                .unwrap()
        })
        .await
        .expect("confirmed");

    let done = t.reload(booking.id).await;
    assert_eq!(done.status, BookingStatus::Cancelled);
    assert_eq!(done.cancel_code.as_deref(), Some("unpaid"));
    assert_eq!(refunds(&t).await.len(), 1);
    assert_eq!(t.events.cancelled.all().len(), 1);
    // The party is told once that the payment is paid back, and is not sent
    // the "not held" message as well: the payment was on its way.
    assert_eq!(told(&t, "scheduling.late_payment_refunded"), 1);
}

/// Without a gateway no link can be made: the self-booking is refused and its
/// slot goes back at once.
#[renox::test]
async fn a_self_booking_is_refused_when_no_payment_link_can_be_made() {
    let t = boot_real().await;
    let party = patient(&t).await;
    let (doctor, service) = clinic(&t, false).await;
    let start = in_hours(30);

    let refused = self_book(&t, party, &service, &doctor, start, now()).await;

    assert!(matches!(refused, Err(Refusal::Invalid(_))), "{refused:?}");
    let all = Booking::all(t.db()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, BookingStatus::Cancelled);
    assert_eq!(all[0].cancel_code.as_deref(), Some("unpaid"));
    // The time can be booked by staff straight away.
    let again = t.book_for(party, &service, &[&doctor], start).await;
    assert_eq!(again.status, BookingStatus::Booked);
}

/// Through the portal route: the booking is made, awaits its payment and
/// carries the page to pay on.
#[renox::test]
async fn the_portal_route_books_with_the_payment_page() {
    let t = boot_paid().await;
    let party = patient(&t).await;
    let (_doctor, service) = clinic(&t, false).await;
    let sara = t.user("Sara", &[]).await;
    t.portal.allow(sara.id, party);
    let start = wl_scheduling::pages::local_text(in_hours(48));
    let (who, svc) = (party.0.to_string(), service.id.to_string());
    t.app.acting_as(&sara);

    t.app
        .htmx()
        .post(
            "/scheduling/portal/bookings",
            &[
                ("party", who.as_str()),
                ("service", svc.as_str()),
                ("starts_at", start.as_str()),
            ],
        )
        .await
        .assert_ok();

    let all = Booking::all(t.db()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].payment_pending);
    assert!(
        all[0]
            .prepayment_path
            .as_deref()
            .is_some_and(|p| p.starts_with("/pay/"))
    );
    assert_eq!(t.gateway.checkouts().len(), 0, "no payment started yet");
}
