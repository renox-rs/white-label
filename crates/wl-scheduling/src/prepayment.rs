//! The prepayment hold of a self-booking (wl-scheduling §3 *No double-booking*,
//! §5.2 steps 2-3, `SCH.RUL.04`; BL-SCH-10).
//!
//! A service type with a `prepayment_amount` asks the party who books by
//! themselves to pay in advance. The booking (`requested` when the service
//! needs acceptance, else `booked`) holds its slot while the payment is
//! awaited (`payment_pending`), and a payment link of `wl-payments` is made
//! for it with the expiry **equal to the hold** (15 minutes by default), so
//! that one timer ends it:
//!
//! - `PaymentLinkExpired` for the booking's link, or `SCH.RUL.04`
//!   ([`release_due`], in the five-minute run of [`crate::rules`]) when the
//!   hold time has passed and the booking is still unpaid, whichever comes
//!   first, cancels the booking as the system with the reason `unpaid`
//!   (initiated by the party, nothing to forfeit or refund). The slot is
//!   released to the waiting list ([`crate::waiting::offer_released`]) and the
//!   party is told. The second trigger finds the booking cancelled and does
//!   nothing.
//! - `PaymentFailed` ends nothing: the party retries on the same link until
//!   it expires, so the module does not listen to it.
//! - `PaymentReceived` for the booking while the hold lasts keeps the booking
//!   (no longer awaiting payment). A payment that arrives after the hold
//!   ended is refunded in full once, whether or not the slot is still free,
//!   with the reason *slot not available*; the cancelled booking is not
//!   revived and the party is told to book again.
//!
//! Decisions:
//!
//! - **Which booking.** `PaymentReceived` does not name the booking, so the
//!   module asks `PaymentRecorder::payments_for(booking)` for the bookings of
//!   the paying party that are awaiting payment or were cancelled.
//! - **The amount of the link** is the service type's `prepayment_amount`;
//!   the booking deposit is a separate setting that staff-made bookings take
//!   as today. When both are set the prepayment is taken to cover the deposit
//!   (the booking holds the deposit once it is paid).
//! - **A request** that awaits payment is held until the link expires: the
//!   request hold does not shorten it. Staff `accept` while unpaid keeps the
//!   unpaid hold; once paid, a `requested` booking holds for the request
//!   hold time from the payment.
//! - **No link.** When payments cannot make a link (the module is off, no
//!   gateway, a refusal) the booking is cancelled as `unpaid` at once and the
//!   booker is told the time cannot be paid for.
//! - **One refund per payment.** A row per payment ([`PrepaymentRefund`],
//!   unique) is the claim; a second `PaymentReceived` for the same payment
//!   finds it and does nothing. The refund is asked from the payment, so a
//!   refund `wl-payments` asked already for the same money (its own late
//!   confirmation) leaves nothing to ask. When `wl-payments` refuses, the
//!   claim is given back and the front desk gets a task.

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::payments::{
    DEFAULT_CURRENCY, PAYMENT_LINK_SUBJECT_TYPE, PaymentLinkExpired, PaymentLinkRequest,
    PaymentReceived, RefundRequest, RefundSource, payment_links, payment_recorder,
};
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{LegalEntity, Subject};

use crate::booking::{Booking, BookingStatus, UNPAID};
use crate::config::{SchedulingConfig, config_of};
use crate::notices;
use crate::refusal::Refusal;
use crate::resource::ServiceType;
use crate::rules;
use crate::service::{self, Actor, CancelRequest};
use crate::texts::english;

/// The refund asked of a payment that arrived after its booking was released.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_prepayment_refunds", index(booking_id))]
pub struct PrepaymentRefund {
    /// The primary key.
    pub id: i64,
    /// The released booking.
    #[model(references = "wl_scheduling_bookings")]
    pub booking_id: i64,
    /// The payment (`wl-payments`); one row per payment.
    pub payment_id: i64,
    /// The amount, in fils.
    pub amount: i64,
    /// The refund asked of `wl-payments`; empty while the request is on its way.
    pub refund_id: Option<i64>,
    /// When it was claimed.
    pub created_at: Option<DateTime>,
}

/// What a self-booking of `service` must pay in advance, in fils; 0 for none.
pub fn amount_due(service: &ServiceType) -> i64 {
    service.prepayment_amount.max(0)
}

/// Until when a self-booking made at `at` holds its slot while unpaid: the
/// prepayment hold, and for a request not less than its request hold.
pub fn hold_until(
    at: DateTime,
    config: &SchedulingConfig,
    request_hold: Option<DateTime>,
) -> DateTime {
    let until = at + renox::chrono::TimeDelta::minutes(config.prepayment_hold_minutes);
    request_hold.map_or(until, |hold| hold.max(until))
}

/// Whether the booking still awaits its prepayment: `requested` or `booked`
/// and unpaid.
pub fn awaits_payment(booking: &Booking) -> bool {
    booking.payment_pending
        && matches!(
            booking.status,
            BookingStatus::Requested | BookingStatus::Booked
        )
}

/// Makes the payment link of the prepayment `amount` for the booking just
/// made, expiring when its hold does, and stores it on the booking. When no
/// link can be made the booking is cancelled as `unpaid` and the booker is
/// refused.
///
/// # Errors
///
/// `Refusal::Invalid` when payments cannot make the link.
pub(crate) async fn open_link(
    state: &AppState,
    booking: &Booking,
    amount: i64,
    at: DateTime,
) -> std::result::Result<Booking, Refusal> {
    let entity = LegalEntity::default_entity(&state.db).await?;
    let expires_at = booking.hold_until.unwrap_or(at);
    let request = PaymentLinkRequest {
        party: booking.party(),
        legal_entity_id: entity.id,
        amount,
        currency: DEFAULT_CURRENCY.to_owned(),
        prepaid: booking.subject(),
        expires_at,
        created_by: None,
    };
    match payment_links(state).create(state, request).await {
        Ok(Some(link)) => {
            Booking::query()
                .where_(Booking::ID.eq(booking.id))
                .update(
                    &state.db,
                    &[
                        ("prepayment_type", &Some(link.link.subject_type.clone())),
                        ("prepayment_id", &Some(link.link.subject_id)),
                        ("prepayment_path", &Some(link.path.clone())),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Ok(Booking::find_or_404(&state.db, booking.id).await?)
        }
        Ok(None) | Err(_) => {
            // Nothing was paid for: the slot goes back at once.
            cancel_unpaid(state, booking.id, at, false).await?;
            Err(Refusal::invalid(
                "wl_scheduling.problem.payment_unavailable",
            ))
        }
    }
}

/// Cancels booking `id` as the system with the reason `unpaid` if it still
/// awaits its prepayment; the party is told when `tell` is set. Returns
/// whether this call cancelled it: the other trigger (the link's expiry or
/// `SCH.RUL.04`) finds it cancelled and does nothing.
///
/// # Errors
///
/// A database error or the notifier's, or a refusal that is not "someone
/// else was first".
pub async fn cancel_unpaid(state: &AppState, id: i64, at: DateTime, tell: bool) -> Result<bool> {
    let Some(booking) = Booking::find(&state.db, id).await? else {
        return Ok(false);
    };
    if !awaits_payment(&booking) {
        return Ok(false);
    }
    match service::cancel(
        state,
        booking.id,
        CancelRequest::new(UNPAID),
        Actor::system(),
        at,
    )
    .await
    {
        Ok(done) => {
            withdraw_link(state, &done).await;
            if tell {
                notices::unpaid_released(state, &done).await?;
            }
            Ok(true)
        }
        Err(Refusal::Transition(TransitionError::NotFrom(_) | TransitionError::Conflict)) => {
            Ok(false)
        }
        Err(other) => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            other.plain(),
        )),
    }
}

/// Closes the payment link of the released `booking`, so a party who still has
/// the page open cannot pay for a slot that is gone (`SCH.RUL.04`). A link
/// that already expired (the link-expiry path) or was paid is not open and is
/// left as it is. The booking stays cancelled whatever happens here: a failure
/// is logged, and a payment that still arrives is refunded.
async fn withdraw_link(state: &AppState, booking: &Booking) {
    let (Some(subject_type), Some(subject_id)) =
        (booking.prepayment_type.clone(), booking.prepayment_id)
    else {
        return;
    };
    if subject_type != PAYMENT_LINK_SUBJECT_TYPE {
        return;
    }
    let link = Subject {
        subject_type,
        subject_id,
    };
    if let Err(error) = payment_links(state).cancel(state, &link).await {
        eprintln!(
            "scheduling: the payment link of booking {} was not withdrawn: {error}",
            booking.number
        );
    }
}

/// `PaymentLinkExpired`: cancels the unpaid booking the link was made for.
/// A link of a booking that is paid (or has another link, or is not a
/// booking's) is ignored.
///
/// # Errors
///
/// As [`cancel_unpaid`].
pub async fn on_link_expired(
    state: &AppState,
    event: &PaymentLinkExpired,
    at: DateTime,
) -> Result<bool> {
    let Some(prepaid) = &event.prepaid else {
        return Ok(false);
    };
    if prepaid.subject_type != BOOKING_SUBJECT_TYPE {
        return Ok(false);
    }
    let Some(booking) = Booking::find(&state.db, prepaid.subject_id).await? else {
        return Ok(false);
    };
    if booking
        .prepayment_id
        .is_some_and(|id| id != event.payment_link.subject_id)
    {
        return Ok(false);
    }
    cancel_unpaid(state, booking.id, at, true).await
}

/// `SCH.RUL.04`: cancels the bookings whose prepayment hold has passed at
/// `at` and that are still unpaid. Returns how many were cancelled.
///
/// # Errors
///
/// As [`cancel_unpaid`].
pub async fn release_due(state: &AppState, at: DateTime) -> Result<usize> {
    let mut released = 0;
    for status in [BookingStatus::Requested, BookingStatus::Booked] {
        let due = Booking::query()
            .where_(Booking::STATUS.eq(status))
            .where_(Booking::PAYMENT_PENDING.eq(true))
            .where_(Booking::HOLD_UNTIL.lte(at))
            .order_by(Booking::ID)
            .get(&state.db)
            .await?;
        for booking in due {
            if cancel_unpaid(state, booking.id, at, true).await? {
                released += 1;
            }
        }
    }
    Ok(released)
}

/// The booking the payment of `event` was made in advance for: among the
/// paying party's bookings that await payment or were cancelled, the one whose
/// prepaid payments include it, with that payment's own currency.
async fn booking_of(
    state: &AppState,
    event: &PaymentReceived,
) -> Result<Option<(Booking, String)>> {
    let candidates = Booking::query()
        .where_(Booking::PARTY_ID.eq(event.party.0))
        .where_(Booking::PREPAYMENT_TYPE.eq(PAYMENT_LINK_SUBJECT_TYPE))
        .order_by(Booking::ID)
        .get(&state.db)
        .await?;
    let recorder = payment_recorder(state);
    for booking in candidates {
        if !(awaits_payment(&booking) || booking.status == BookingStatus::Cancelled) {
            continue;
        }
        let paid = recorder.payments_for(&state.db, &booking.subject()).await?;
        if let Some(payment) = paid.iter().find(|payment| payment.id == event.payment) {
            return Ok(Some((booking, payment.currency.clone())));
        }
    }
    Ok(None)
}

/// `PaymentReceived`: keeps the booking the money was paid for while its hold
/// lasts; refunds the payment in full, once, when the booking was released
/// before it arrived.
///
/// # Errors
///
/// A database error or the notifier's.
pub async fn on_payment_received(
    state: &AppState,
    event: &PaymentReceived,
    at: DateTime,
) -> Result<()> {
    let Some((booking, currency)) = booking_of(state, event).await? else {
        return Ok(());
    };
    if awaits_payment(&booking) && booking.hold_until.is_none_or(|until| until > at) {
        return confirm_payment(state, &booking, at).await;
    }
    if awaits_payment(&booking) {
        // The hold passed and nobody has released it yet: it is released now
        // and the money goes back, as the slot may be gone.
        cancel_unpaid(state, booking.id, at, false).await?;
    }
    let booking = Booking::find_or_404(&state.db, booking.id).await?;
    if booking.status != BookingStatus::Cancelled {
        return Ok(());
    }
    refund_late(state, &booking, event, &currency, at).await
}

/// The payment arrived in time: the booking no longer awaits it. A `booked`
/// booking holds on its own; a `requested` one for the request hold time.
async fn confirm_payment(state: &AppState, booking: &Booking, at: DateTime) -> Result<()> {
    let hold: Option<DateTime> = (booking.status == BookingStatus::Requested)
        .then(|| at + renox::chrono::TimeDelta::minutes(config_of(state).request_hold_minutes));
    Booking::query()
        .where_(Booking::ID.eq(booking.id))
        .where_(Booking::PAYMENT_PENDING.eq(true))
        .update(
            &state.db,
            &[
                ("payment_pending", &false),
                ("hold_until", &hold),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Asks the full refund of the late payment, in the currency it was paid in,
/// once, and tells the party.
async fn refund_late(
    state: &AppState,
    booking: &Booking,
    event: &PaymentReceived,
    currency: &str,
    at: DateTime,
) -> Result<()> {
    let mut claim = PrepaymentRefund {
        booking_id: booking.id,
        payment_id: event.payment.0,
        amount: event.amount,
        created_at: Some(at),
        ..PrepaymentRefund::default()
    };
    if let Err(error) = claim.save(&state.db).await {
        // A second `PaymentReceived` for the same payment: already claimed.
        let claimed = PrepaymentRefund::query()
            .where_(PrepaymentRefund::PAYMENT_ID.eq(event.payment.0))
            .exists(&state.db)
            .await?;
        return if claimed { Ok(()) } else { Err(error) };
    }
    let entity = LegalEntity::default_entity(&state.db).await?;
    let request = RefundRequest {
        source: RefundSource::Payment(event.payment),
        party: event.party,
        legal_entity_id: entity.id,
        amount: event.amount,
        currency: currency.to_owned(),
        method: None,
        gateway_txn: None,
        reason: english("wl_scheduling.prepayment.refund_reason", &[]),
    };
    match payment_recorder(state)
        .request_refund(state, request, None)
        .await
    {
        Ok(refund) => {
            PrepaymentRefund::query()
                .where_(PrepaymentRefund::PAYMENT_ID.eq(event.payment.0))
                .update(&state.db, &[("refund_id", &Some(refund.0))])
                .await?;
            notices::late_payment(state, booking).await
        }
        Err(refused) => {
            // Give the claim back and make the refund somebody's task.
            PrepaymentRefund::query()
                .where_(PrepaymentRefund::PAYMENT_ID.eq(event.payment.0))
                .delete(&state.db)
                .await?;
            eprintln!(
                "scheduling: the refund of payment {} for booking {} was refused: {refused}",
                event.payment, booking.number
            );
            refund_task(state, booking, event).await
        }
    }
}

/// A task for the front desk: the late payment of `booking` could not be
/// refunded by the module.
async fn refund_task(state: &AppState, booking: &Booking, event: &PaymentReceived) -> Result<()> {
    let creator = task_creator(state);
    let key = format!("scheduling.late_payment:{}", event.payment);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let title = english(
        "wl_scheduling.prepayment.task_title",
        &[("number", &booking.number)],
    );
    let description = english(
        "wl_scheduling.prepayment.task_body",
        &[("number", &booking.number), ("payment", &event.payment)],
    );
    let mut ticket = OpenTicket::new(booking.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key);
    if let Some(user) = rules::front_desk(state, booking).await?.first() {
        ticket = ticket.assign_to(Assignee::user(*user));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Registers the model and the listeners of the link's expiry and the
/// payments received.
pub(crate) fn register(app: &mut Registry) {
    app.model::<PrepaymentRefund>();
    app.listen(|event: PaymentLinkExpired, state: AppState| async move {
        on_link_expired(&state, &event, now()).await.map(|_| ())
    });
    app.listen(|event: PaymentReceived, state: AppState| async move {
        on_payment_received(&state, &event, now()).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: u32) -> DateTime {
        "2026-11-02T10:00:00Z".parse::<DateTime>().unwrap()
            + renox::chrono::TimeDelta::minutes(i64::from(minute))
    }

    #[test]
    fn the_hold_is_the_prepayment_hold_and_a_request_keeps_the_longer() {
        let config = SchedulingConfig::default();
        assert_eq!(hold_until(at(0), &config, None), at(15));
        assert_eq!(hold_until(at(0), &config, Some(at(10))), at(15));
        assert_eq!(hold_until(at(0), &config, Some(at(30))), at(30));
    }

    #[test]
    fn only_an_unpaid_requested_or_booked_booking_awaits_payment() {
        let unpaid = |status| Booking {
            status,
            payment_pending: true,
            ..Booking::default()
        };
        assert!(awaits_payment(&unpaid(BookingStatus::Booked)));
        assert!(awaits_payment(&unpaid(BookingStatus::Requested)));
        assert!(!awaits_payment(&unpaid(BookingStatus::Cancelled)));
        assert!(!awaits_payment(&Booking {
            status: BookingStatus::Booked,
            payment_pending: false,
            ..Booking::default()
        }));
    }

    #[test]
    fn a_service_without_a_prepayment_asks_nothing() {
        let mut service = ServiceType::new("consultation", "Consultation", 30);
        assert_eq!(amount_due(&service), 0);
        service.prepayment_amount = 15_000;
        assert_eq!(amount_due(&service), 15_000);
    }
}
