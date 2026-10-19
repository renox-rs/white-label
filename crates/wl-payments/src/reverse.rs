//! Reversing a payment (wl-payments §4 `reverse`, A8-A10).
//!
//! `reverse` (a user with `payments.reverse`) and `reverse_by_system` (for
//! another module's call through the kernel, with or without a user id)
//! move a `received` payment to `reversed`. A reason is required: the
//! [`ReversalReason`] (`entered_in_error` or `bank_return`; `chargeback`
//! belongs to the chargeback flow) with an optional note. A gateway payment
//! with an open dispute is refused: the service asks the
//! [`Gateway`](crate::gateway::Gateway) before the transition and passes the
//! answer in the input; the stored dispute is checked by the guard.
//!
//! The effects, in the transition's transaction: every active allocation
//! gets `removed_at`, the removal reason and the user (the rows stay), and
//! the reversal reason, note, user and day (today in Asia/Dubai) are stored
//! on the payment; its date is kept. The unallocated credit goes with the
//! payment, because the credit balance counts only `received` payments. The
//! module never changes a target's state: after the commit the listener
//! ([`crate::events`]) emits `PaymentReversed`, and [`after_reversal`] tells
//! the party of a bank return and checks for a credit deficit.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{Direction, PaymentId, PaymentStatus, ReversalReason};

use crate::allocation::Allocation;
use crate::followup;
use crate::gateway::gateway_of;
use crate::notices;
use crate::payment::Payment;
use crate::permissions;

/// The `reverse` transition's name.
pub const REVERSE: &str = "reverse";

/// The `reverse_by_system` transition's name.
pub const REVERSE_BY_SYSTEM: &str = "reverse_by_system";

/// The message of a refused reversal while a dispute is open.
pub const DISPUTE_MESSAGE: &str = "This payment has an open dispute; use the chargeback flow";

/// What a reverse run carries to its guards and effects, gathered by
/// [`reverse`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReverseInput {
    /// Why; `None` is refused.
    pub reason: Option<ReversalReason>,
    /// The note given with it.
    pub note: Option<String>,
    /// The user who reverses it; `None` for the system.
    pub by: Option<i64>,
    /// Whether the gateway reports an open dispute.
    pub gateway_dispute: bool,
}

fn input(ctx: &TransitionCtx) -> Option<ReverseInput> {
    ctx.input_as::<ReverseInput>().ok()
}

/// Whether the stored dispute of `payment` is open.
pub(crate) fn stored_dispute_open(payment: &Payment) -> bool {
    payment
        .dispute_status
        .as_deref()
        .is_some_and(|status| status.trim().eq_ignore_ascii_case("open"))
}

/// The text of the transition's reason: the note, else the reason code.
fn reason_text(reason: ReversalReason, note: Option<&str>) -> String {
    note.map(str::trim)
        .filter(|note| !note.is_empty())
        .map_or_else(|| reason.as_str().to_owned(), str::to_owned)
}

/// `transition` with the guards and effects both reverse transitions share.
fn with_rules(transition: Transition<Payment>) -> Transition<Payment> {
    transition
        .from([PaymentStatus::Received])
        .to(PaymentStatus::Reversed)
        .reason_required()
        .guard("reason_given", |_payment: &Payment, ctx| {
            let reason = input(ctx).and_then(|input| input.reason);
            Box::pin(async move {
                match reason {
                    None => Err("A reversal needs a reason.".to_owned()),
                    Some(reason) if !reason.is_manual() => Err(
                        "A chargeback is not reversed here; use the chargeback flow.".to_owned(),
                    ),
                    Some(_) => Ok(()),
                }
            })
        })
        .guard("no_open_dispute", |payment: &Payment, ctx| {
            let open =
                stored_dispute_open(payment) || input(ctx).is_some_and(|i| i.gateway_dispute);
            Box::pin(async move {
                if open {
                    Err(DISPUTE_MESSAGE.to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|payment: &Payment, ctx, tx| {
            let id = payment.id;
            let input = input(ctx).unwrap_or_default();
            Box::pin(async move {
                let reason = input.reason.unwrap_or_default();
                let now = renox::db::now();
                // The rows stay; they are marked removed.
                Allocation::query()
                    .where_(Allocation::PAYMENT_ID.eq(id))
                    .where_(Allocation::REMOVED_AT.is_null())
                    .update(
                        &mut *tx,
                        &[
                            ("removed_at", &Some(now)),
                            ("removed_reason", &Some(reason.as_str().to_owned())),
                            ("removed_by", &input.by),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                let note = input
                    .note
                    .as_deref()
                    .map(str::trim)
                    .filter(|note| !note.is_empty())
                    .map(str::to_owned);
                // Not the status column, so outside the machine's guard.
                Payment::query()
                    .where_(Payment::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("reversal_reason", &Some(reason.as_str().to_owned())),
                            ("reversal_note", &note),
                            ("reversed_by", &input.by),
                            ("reversed_on", &Some(today())),
                            ("updated_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// `reverse` and `reverse_by_system`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Payment>> {
    vec![
        with_rules(
            Transition::new(REVERSE)
                .label("Reverse")
                .tone("danger")
                .permission(permissions::REVERSE),
        ),
        with_rules(
            Transition::new(REVERSE_BY_SYSTEM)
                .label("Reverse")
                .tone("danger")
                .system(),
        ),
    ]
}

/// Reverses `payment` for `reason` and `note`: by `actor` (a user with
/// `payments.reverse`), or by the system when there is none, in which case
/// `by` is the user id to store (the calling module has authorized its
/// user). `PaymentReversed` follows after the commit, then the party's
/// notice and the credit-deficit check ([`after_reversal`]).
///
/// # Errors
///
/// `Forbidden` for a user without `payments.reverse`. `NotFrom` unless the
/// payment is `received` (a second reverse). `Guard` (422) without a
/// reason, for `chargeback`, or while a dispute is open (stored or at the
/// gateway); nothing is written then.
pub async fn reverse(
    state: &AppState,
    actor: Option<&AuthUser>,
    by: Option<i64>,
    payment: &Payment,
    reason: Option<ReversalReason>,
    note: Option<&str>,
) -> std::result::Result<Payment, TransitionError> {
    let by = actor.map(|user| user.id).or(by);
    let gateway_dispute = match payment.gateway_transaction.as_deref() {
        Some(txn) if !txn.trim().is_empty() => gateway_of(state)
            .open_dispute(txn)
            .await
            .map_err(TransitionError::Failed)?
            .is_some(),
        _ => false,
    };
    let input = ReverseInput {
        reason,
        note: note.map(str::to_owned),
        by,
        gateway_dispute,
    };
    let builder = payment.transition(if actor.is_some() {
        REVERSE
    } else {
        REVERSE_BY_SYSTEM
    });
    let mut builder = match actor {
        Some(user) => builder.by(user),
        None => builder.by_system(),
    };
    if let Some(reason) = reason {
        builder = builder.reason(reason_text(reason, note));
    }
    let _outcome = builder.input(&input).run(state).await?;
    Payment::find_or_404(&state.db, payment.id)
        .await
        .map_err(TransitionError::Failed)
}

/// [`reverse`] by payment id, for the kernel's `PaymentRecorder`.
pub(crate) async fn reverse_by_id(
    state: &AppState,
    id: PaymentId,
    reason: ReversalReason,
    note: Option<String>,
    by: Option<i64>,
) -> std::result::Result<Payment, TransitionError> {
    let payment = Payment::find_or_404(&state.db, id.0)
        .await
        .map_err(TransitionError::Failed)?;
    reverse(state, None, by, &payment, Some(reason), note.as_deref()).await
}

/// Reverses the `received` payment `payment` in `tx` as the system, for the
/// failure of the refund it paid out (the reason text is `note`). `by` is
/// the user to store. Emit the outcome after the commit.
pub(crate) async fn reverse_in(
    tx: &mut renox::db::Transaction,
    payment: &Payment,
    reason: ReversalReason,
    note: &str,
    by: Option<i64>,
) -> std::result::Result<renox::state_machine::Outcome<Payment>, TransitionError> {
    let input = ReverseInput {
        reason: Some(reason),
        note: Some(note.to_owned()),
        by,
        gateway_dispute: false,
    };
    payment
        .transition(REVERSE_BY_SYSTEM)
        .by_system()
        .reason(reason_text(reason, Some(note)))
        .input(&input)
        .run_in(tx)
        .await
}

/// Whether a committed reversal of `payment` is followed by the party's
/// bank-return notice and the credit-deficit check: only for money in. An
/// outgoing refund payment is told by the refund notices instead.
pub(crate) fn has_follow_up(payment: &Payment) -> bool {
    payment.direction == Direction::In
}

/// What follows a committed reversal of `payment`: the party's bank-return
/// notice, then the credit-deficit check (A9, A10). Nothing for an outgoing
/// payment (a refund's voucher).
pub(crate) async fn after_reversal(state: &AppState, payment: &Payment) -> Result {
    if !has_follow_up(payment) {
        return Ok(());
    }
    if payment.reversal_reason == Some(ReversalReason::BankReturn) {
        notices::payment_returned(state, payment).await?;
    }
    followup::check_deficit(state, payment).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_incoming_payments_have_a_return_notice_and_a_deficit_check() {
        let incoming = Payment::default();
        assert_eq!(incoming.direction, Direction::In);
        assert!(has_follow_up(&incoming));
        let outgoing = Payment {
            direction: Direction::Out,
            ..Default::default()
        };
        assert!(!has_follow_up(&outgoing));
    }

    fn run_input(reason: Option<ReversalReason>, gateway_dispute: bool) -> TransitionCtx {
        let input = ReverseInput {
            reason,
            note: None,
            by: Some(3),
            gateway_dispute,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    fn received() -> Payment {
        Payment {
            status: PaymentStatus::Received,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_received_payment_is_reversed() {
        let machine = Payment::machine();
        for name in [REVERSE, REVERSE_BY_SYSTEM] {
            let transition = machine.find(name).expect("transition");
            assert!(
                transition.check_from(PaymentStatus::Received).is_ok(),
                "{name}"
            );
            for state in [
                PaymentStatus::Pending,
                PaymentStatus::Reversed,
                PaymentStatus::Failed,
                PaymentStatus::ChargedBack,
            ] {
                assert!(transition.check_from(state).is_err(), "{name} {state:?}");
            }
        }
    }

    #[renox::test]
    async fn a_reason_is_required_and_a_chargeback_is_not_a_manual_one() {
        let machine = Payment::machine();
        let transition = machine.find(REVERSE).expect("transition");
        for reason in [ReversalReason::EnteredInError, ReversalReason::BankReturn] {
            assert!(
                transition
                    .check_guards(&received(), &run_input(Some(reason), false))
                    .await
                    .is_ok()
            );
        }
        for reason in [None, Some(ReversalReason::Chargeback)] {
            assert!(matches!(
                transition
                    .check_guards(&received(), &run_input(reason, false))
                    .await,
                Err(TransitionError::Guard(_))
            ));
        }
    }

    #[renox::test]
    async fn an_open_dispute_refuses_the_reversal_with_the_chargeback_message() {
        let machine = Payment::machine();
        let transition = machine.find(REVERSE_BY_SYSTEM).expect("transition");
        let reason = Some(ReversalReason::BankReturn);
        // The gateway reports one.
        let refused = transition
            .check_guards(&received(), &run_input(reason, true))
            .await;
        assert!(matches!(&refused, Err(TransitionError::Guard(m)) if m.contains(DISPUTE_MESSAGE)));
        // The stored one is open.
        let disputed = Payment {
            dispute_status: Some("open".into()),
            ..received()
        };
        assert!(
            transition
                .check_guards(&disputed, &run_input(reason, false))
                .await
                .is_err()
        );
        // A dispute that is closed does not block.
        let closed = Payment {
            dispute_status: Some("won".into()),
            ..received()
        };
        assert!(
            transition
                .check_guards(&closed, &run_input(reason, false))
                .await
                .is_ok()
        );
    }

    #[test]
    fn the_history_reason_is_the_note_else_the_code() {
        assert_eq!(
            reason_text(ReversalReason::BankReturn, Some(" bounced ")),
            "bounced"
        );
        assert_eq!(
            reason_text(ReversalReason::BankReturn, Some("  ")),
            "bank_return"
        );
        assert_eq!(
            reason_text(ReversalReason::EnteredInError, None),
            "entered_in_error"
        );
    }

    #[test]
    fn a_stored_dispute_is_open_only_with_that_status() {
        assert!(!stored_dispute_open(&received()));
        let mut p = received();
        p.dispute_status = Some(" Open ".into());
        assert!(stored_dispute_open(&p));
        p.dispute_status = Some("lost".into());
        assert!(!stored_dispute_open(&p));
    }
}
