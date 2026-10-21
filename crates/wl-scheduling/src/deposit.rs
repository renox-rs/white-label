//! The booking deposit (wl-scheduling §4.1 `cancel`, `mark_no_show`,
//! `reschedule`, §5.2, `SCH.ACL.05`).
//!
//! A service type may take a deposit when the booking is made. It is kept
//! only when the party is at fault:
//!
//! | Event | Deposit |
//! |---|---|
//! | the business cancels | refunded in full |
//! | the party cancels outside the forfeit window | refunded in full |
//! | the party cancels inside the forfeit window (*late*) | forfeited |
//! | no-show | forfeited |
//! | a reschedule, by the business or by the party before the cut-off | carried to the new time: the booking keeps it |
//!
//! A refund is asked of `wl-payments` through the kernel's
//! [`RefundRequest`] (source: the booking); this module records what it
//! decided and the refund's id. The refund is paid after another user
//! approved it, and a refund to a blocked party waits (`PTY.ACL.03`): both
//! belong to `wl-payments`. A user with `bookings.waive_deposit` turns a
//! forfeiture into a refund with a reason ([`waive`]).
//!
//! Decisions:
//!
//! - The deposit is *held* when the service type took one and its payment is
//!   not awaited ([`Booking::deposit_held`]); otherwise there is nothing to
//!   settle and the booking shows no deposit outcome.
//! - The deposit is settled after the cancellation or the no-show committed:
//!   a refund that `wl-payments` refuses does not undo the cancellation. The
//!   service reports the refusal, the booking shows no outcome, and
//!   [`settle`] is safe to run again (it settles a booking once).
//! - The deposit is claimed (the outcome column) before the refund is asked
//!   and the refund's id is written after it. A process that dies between the
//!   two leaves a *claimed* booking: an outcome with a refund amount and no
//!   refund id. [`settle`] resumes it ([`unrefunded`]): it asks the refund
//!   for the claimed amount and writes the id, and a booking with an id is
//!   never asked again.
//! - The currency is `AED` and the refund is asked of the default legal
//!   entity.
//! - A reschedule needs no action here: the booking is the same row at its
//!   new time, so the deposit it holds is carried. `BookingRescheduled` says
//!   how much.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::payments::{RefundRequest, RefundSource, payment_recorder};
use wl_kernel::scheduling::{CancelStage, DepositOutcome, Initiator};

use crate::booking::Booking;
use crate::refusal::Refusal;
use crate::resource::ServiceType;
use crate::service::Actor;
use crate::texts::english;

/// What became of a deposit, in fils. The three parts add up to the deposit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settlement {
    /// Applied to the work done.
    pub applied: i64,
    /// Paid back to the party.
    pub refund: i64,
    /// Kept by the business.
    pub forfeited: i64,
}

impl Settlement {
    /// The whole deposit is paid back.
    pub fn refunded(deposit: i64) -> Self {
        Settlement {
            refund: deposit,
            ..Settlement::default()
        }
    }

    /// The whole deposit is kept.
    pub fn forfeit(deposit: i64) -> Self {
        Settlement {
            forfeited: deposit,
            ..Settlement::default()
        }
    }

    /// The outcome to show: work done first, then a forfeit, else a refund;
    /// `None` when there was no deposit.
    pub fn outcome(&self) -> Option<DepositOutcome> {
        if self.applied > 0 {
            Some(DepositOutcome::AppliedToWorkDone)
        } else if self.forfeited > 0 {
            Some(DepositOutcome::Forfeited)
        } else if self.refund > 0 {
            Some(DepositOutcome::Refunded)
        } else {
            None
        }
    }
}

/// What a cancellation does to a `deposit` of fils (`SCH.ACL.05`,
/// `SCH.ACL.06`):
///
/// - The business pays the deposit back in full, whenever it cancels, and
///   never applies it to work done: if the party accepts the work, the
///   vertical bills it separately.
/// - Before the start the party loses it only when the cancellation is
///   `late`.
/// - After the check-in, a party-initiated cancellation (`walked_out`,
///   `party_request`) applies the deposit to the work done first, up to the
///   work's price; what is left is forfeited when the service type's
///   walk-out setting says so, else paid back.
pub fn decide_cancel(
    deposit: i64,
    initiated_by: Initiator,
    late: bool,
    stage: CancelStage,
    work_done: i64,
    walk_out_forfeits: bool,
) -> Settlement {
    if deposit <= 0 {
        return Settlement::default();
    }
    match (initiated_by, stage) {
        (Initiator::Business, _) => Settlement::refunded(deposit),
        (Initiator::Party, CancelStage::BeforeStart) if late => Settlement::forfeit(deposit),
        (Initiator::Party, CancelStage::BeforeStart) => Settlement::refunded(deposit),
        (Initiator::Party, CancelStage::CheckedIn | CancelStage::InProgress) => {
            let applied = work_done.clamp(0, deposit);
            let rest = deposit - applied;
            Settlement {
                applied,
                refund: if walk_out_forfeits { 0 } else { rest },
                forfeited: if walk_out_forfeits { rest } else { 0 },
            }
        }
    }
}

/// Asks `wl-payments` to pay `amount` of the deposit of `booking` back, by
/// user `by` (`None` = the system), and returns the refund's id.
async fn request_refund(
    state: &AppState,
    booking: &Booking,
    amount: i64,
    reason_key: &str,
    by: Option<i64>,
) -> std::result::Result<i64, Refusal> {
    let entity = LegalEntity::default_entity(&state.db).await?;
    let reason = english(reason_key, &[("number", &booking.number)]);
    let request = RefundRequest {
        source: RefundSource::Booking(booking.subject()),
        party: booking.party(),
        legal_entity_id: entity.id,
        amount,
        currency: "AED".to_owned(),
        method: None,
        gateway_txn: None,
        reason,
    };
    let id = payment_recorder(state)
        .request_refund(state, request, by)
        .await?;
    Ok(id.0)
}

/// Whether the deposit of `booking` was claimed for a refund that was never
/// asked (or whose id was never written): an outcome with an amount to pay
/// back and no refund id. [`settle`] resumes it.
pub fn unrefunded(booking: &Booking) -> bool {
    booking.deposit_outcome.is_some()
        && booking.deposit_refunded > 0
        && booking.deposit_refund_id.is_none()
}

/// Asks the refund of a claimed-but-unrefunded deposit and writes its id.
/// A waived deposit is asked under the waiver's reason and user.
async fn resume(
    state: &AppState,
    booking: &Booking,
    by: Option<i64>,
) -> std::result::Result<Booking, Refusal> {
    let (reason_key, by) = match booking.deposit_waived_by {
        Some(waiver) => ("wl_scheduling.deposit.refund_waived", Some(waiver)),
        None => ("wl_scheduling.deposit.refund_cancelled", by),
    };
    let id = request_refund(state, booking, booking.deposit_refunded, reason_key, by).await?;
    Booking::query()
        .where_(Booking::ID.eq(booking.id))
        .where_(Booking::DEPOSIT_REFUND_ID.is_null())
        .update(&state.db, &[("deposit_refund_id", &Some(id))])
        .await?;
    Ok(Booking::find_or_404(&state.db, booking.id).await?)
}

/// Records `settlement` on the booking and asks the refund it needs. The
/// deposit is claimed first (the outcome column is set only while empty), so
/// two runs ask one refund.
async fn apply(
    state: &AppState,
    booking: &Booking,
    settlement: Settlement,
    reason_key: &str,
    by: Option<i64>,
) -> std::result::Result<Booking, Refusal> {
    let Some(outcome) = settlement.outcome() else {
        return Ok(booking.clone());
    };
    let claimed = Booking::query()
        .where_(Booking::ID.eq(booking.id))
        .where_(Booking::DEPOSIT_OUTCOME.is_null())
        .update(
            &state.db,
            &[
                ("deposit_outcome", &Some(outcome.as_str().to_owned())),
                ("deposit_applied", &settlement.applied),
                ("deposit_refunded", &settlement.refund),
                ("deposit_forfeited", &settlement.forfeited),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(Booking::find_or_404(&state.db, booking.id).await?);
    }
    if settlement.refund > 0 {
        match request_refund(state, booking, settlement.refund, reason_key, by).await {
            Ok(id) => {
                Booking::query()
                    .where_(Booking::ID.eq(booking.id))
                    .update(&state.db, &[("deposit_refund_id", &Some(id))])
                    .await?;
            }
            Err(refusal) => {
                // Give the claim back: nothing was asked, so the settlement
                // can run again.
                let none: Option<String> = None;
                Booking::query()
                    .where_(Booking::ID.eq(booking.id))
                    .update(
                        &state.db,
                        &[
                            ("deposit_outcome", &none),
                            ("deposit_applied", &0_i64),
                            ("deposit_refunded", &0_i64),
                            ("deposit_forfeited", &0_i64),
                        ],
                    )
                    .await?;
                return Err(refusal);
            }
        }
    }
    Ok(Booking::find_or_404(&state.db, booking.id).await?)
}

/// Settles the deposit of a cancelled booking by who cancelled and whether it
/// was late. A booking without a held deposit, or one already settled, is
/// returned as it is.
///
/// # Errors
///
/// When `wl-payments` refuses the refund (it is off, an unknown party, no
/// default legal entity). The cancellation stays; run [`settle`] again.
pub(crate) async fn settle_cancelled(
    state: &AppState,
    booking: &Booking,
    service: &ServiceType,
    by: Option<i64>,
) -> std::result::Result<Booking, Refusal> {
    let held = booking.deposit_held();
    if held > 0 && unrefunded(booking) {
        return resume(state, booking, by).await;
    }
    if held <= 0 || booking.deposit_outcome.is_some() {
        return Ok(booking.clone());
    }
    let initiated_by = booking
        .cancel_initiated_by
        .as_deref()
        .and_then(Initiator::parse)
        .unwrap_or(Initiator::Business);
    let stage = match booking.cancel_stage.as_deref() {
        Some("checked_in") => CancelStage::CheckedIn,
        Some("in_progress") => CancelStage::InProgress,
        _ => CancelStage::BeforeStart,
    };
    let settlement = decide_cancel(
        held,
        initiated_by,
        booking.cancel_late,
        stage,
        booking.work_done_amount,
        service.walk_out_forfeits,
    );
    apply(
        state,
        booking,
        settlement,
        "wl_scheduling.deposit.refund_cancelled",
        by,
    )
    .await
}

/// A no-show forfeits the deposit held.
pub(crate) async fn settle_no_show(
    state: &AppState,
    booking: &Booking,
) -> std::result::Result<Booking, Refusal> {
    let held = booking.deposit_held();
    if held > 0 && unrefunded(booking) {
        return resume(state, booking, None).await;
    }
    if held <= 0 || booking.deposit_outcome.is_some() {
        return Ok(booking.clone());
    }
    apply(
        state,
        booking,
        Settlement::forfeit(held),
        "wl_scheduling.deposit.refund_cancelled",
        None,
    )
    .await
}

/// Settles the deposit of a booking that is `cancelled` or `no_show` and has
/// no outcome yet, for example because `wl-payments` refused the refund when
/// it was cancelled, or whose claim was never followed by a refund
/// ([`unrefunded`]). Anything else is left alone.
///
/// # Errors
///
/// When `wl-payments` refuses the refund again.
pub async fn settle(
    state: &AppState,
    id: i64,
    actor: Actor,
) -> std::result::Result<Booking, Refusal> {
    let booking = Booking::find_or_404(&state.db, id).await?;
    match booking.status {
        crate::booking::BookingStatus::Cancelled => {
            let service = ServiceType::find_or_404(&state.db, booking.service_type_id).await?;
            settle_cancelled(state, &booking, &service, actor.user).await
        }
        crate::booking::BookingStatus::NoShow => settle_no_show(state, &booking).await,
        _ => Ok(booking),
    }
}

/// Waives the forfeiture of a booking: the deposit the business kept is paid
/// back, with a `reason`, by user `actor` (`bookings.waive_deposit`, checked
/// by the route). Only a forfeited deposit can be waived; a refund is not
/// waived. Audited on the booking: who, when and why.
///
/// # Errors
///
/// A blank reason, a booking with nothing forfeited, a staff-only action done
/// by a party, or a refund `wl-payments` refuses (the forfeiture then stands).
pub async fn waive(
    state: &AppState,
    id: i64,
    reason: &str,
    actor: Actor,
) -> std::result::Result<Booking, Refusal> {
    if actor.kind == crate::booking_machine::ActorKind::Party {
        return Err(Refusal::Transition(
            renox::state_machine::TransitionError::Forbidden,
        ));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(Refusal::invalid("wl_scheduling.problem.reason_needed"));
    }
    let booking = Booking::find_or_404(&state.db, id).await?;
    let amount = booking.deposit_forfeited;
    if amount <= 0 {
        return Err(Refusal::invalid("wl_scheduling.problem.nothing_to_waive"));
    }
    // Claim the forfeiture, so that two users ask one refund.
    let outcome = if booking.deposit_applied > 0 {
        DepositOutcome::AppliedToWorkDone
    } else {
        DepositOutcome::Refunded
    };
    let stamp = now();
    let claimed = Booking::query()
        .where_(Booking::ID.eq(id))
        .where_(Booking::DEPOSIT_FORFEITED.eq(amount))
        .update(
            &state.db,
            &[
                ("deposit_forfeited", &0_i64),
                ("deposit_refunded", &(booking.deposit_refunded + amount)),
                ("deposit_outcome", &Some(outcome.as_str().to_owned())),
                ("deposit_waived_by", &actor.user),
                ("deposit_waived_at", &Some(stamp)),
                ("deposit_waive_reason", &Some(reason.to_owned())),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    if claimed == 0 {
        return Err(Refusal::invalid("wl_scheduling.problem.nothing_to_waive"));
    }
    match request_refund(
        state,
        &booking,
        amount,
        "wl_scheduling.deposit.refund_waived",
        actor.user,
    )
    .await
    {
        Ok(refund) => {
            Booking::query()
                .where_(Booking::ID.eq(id))
                .update(&state.db, &[("deposit_refund_id", &Some(refund))])
                .await?;
        }
        Err(refusal) => {
            let none_user: Option<i64> = None;
            let none_at: Option<renox::db::DateTime> = None;
            let none_reason: Option<String> = None;
            Booking::query()
                .where_(Booking::ID.eq(id))
                .update(
                    &state.db,
                    &[
                        ("deposit_forfeited", &amount),
                        ("deposit_refunded", &booking.deposit_refunded),
                        ("deposit_outcome", &booking.deposit_outcome.clone()),
                        ("deposit_waived_by", &none_user),
                        ("deposit_waived_at", &none_at),
                        ("deposit_waive_reason", &none_reason),
                    ],
                )
                .await?;
            return Err(refusal);
        }
    }
    Ok(Booking::find_or_404(&state.db, id).await?)
}

/// `fils` as `AED 1,234.50`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.abs();
    let whole = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}AED {grouped}.{:02}", fils % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_shown_in_dirhams() {
        assert_eq!(money_text(20_000), "AED 200.00");
        assert_eq!(money_text(123_456_789), "AED 1,234,567.89");
        assert_eq!(money_text(-5), "-AED 0.05");
    }

    const BEFORE: CancelStage = CancelStage::BeforeStart;

    #[test]
    fn the_business_always_pays_the_deposit_back() {
        for stage in [BEFORE, CancelStage::CheckedIn, CancelStage::InProgress] {
            assert_eq!(
                decide_cancel(20_000, Initiator::Business, true, stage, 8_000, true),
                Settlement::refunded(20_000),
                "{stage:?}"
            );
        }
    }

    #[test]
    fn a_party_loses_the_deposit_only_when_late() {
        assert_eq!(
            decide_cancel(20_000, Initiator::Party, false, BEFORE, 0, true),
            Settlement::refunded(20_000)
        );
        assert_eq!(
            decide_cancel(20_000, Initiator::Party, true, BEFORE, 0, true),
            Settlement::forfeit(20_000)
        );
    }

    #[test]
    fn a_walk_out_applies_the_deposit_to_the_work_first() {
        let walk = |work, forfeits| {
            decide_cancel(
                20_000,
                Initiator::Party,
                false,
                CancelStage::InProgress,
                work,
                forfeits,
            )
        };
        // AED 80 of work: AED 80 applied, the AED 120 left follows the setting.
        assert_eq!(
            walk(8_000, true),
            Settlement {
                applied: 8_000,
                refund: 0,
                forfeited: 12_000
            }
        );
        assert_eq!(
            walk(8_000, false),
            Settlement {
                applied: 8_000,
                refund: 12_000,
                forfeited: 0
            }
        );
        // Nothing done: the whole deposit follows the setting.
        assert_eq!(walk(0, false), Settlement::refunded(20_000));
        assert_eq!(walk(0, true), Settlement::forfeit(20_000));
        // More work than deposit: the deposit is applied in full.
        assert_eq!(
            walk(30_000, true),
            Settlement {
                applied: 20_000,
                refund: 0,
                forfeited: 0
            }
        );
    }

    #[test]
    fn no_deposit_settles_nothing() {
        let none = decide_cancel(0, Initiator::Party, true, BEFORE, 0, true);
        assert_eq!(none, Settlement::default());
        assert_eq!(none.outcome(), None);
    }

    #[test]
    fn the_outcome_names_what_happened() {
        assert_eq!(
            Settlement::refunded(5).outcome(),
            Some(DepositOutcome::Refunded)
        );
        assert_eq!(
            Settlement::forfeit(5).outcome(),
            Some(DepositOutcome::Forfeited)
        );
        let mixed = Settlement {
            applied: 3,
            forfeited: 2,
            refund: 0,
        };
        assert_eq!(mixed.outcome(), Some(DepositOutcome::AppliedToWorkDone));
    }
}
