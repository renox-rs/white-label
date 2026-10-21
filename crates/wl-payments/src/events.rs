//! The kernel events the Payment machine emits (wl-payments §6), from a
//! listener on [`Transitioned<Payment>`], so only after a commit and never
//! on a rollback:
//!
//! - `record` / `record_by_system` / `record_set_off` /
//!   `record_set_off_approved` → [`PaymentReceived`] (a new payment has
//!   no allocations yet); money out gives none, except an owner drawing.
//!
//! - `reverse` / `reverse_by_system` → [`PaymentReversed`] with the
//!   allocations the reversal removed, the source, the statement line and
//!   the prepaid record; then the party's bank-return notice and the
//!   credit-deficit check ([`crate::reverse::after_reversal`]). The module
//!   never changes a target's state; each owner reacts to the event. The
//!   notice and the deficit check are for incoming payments only: a refund's
//!   outgoing payment reversed by `fail` is told by the refund notices.
//!
//! - [`InvoiceReopened`] with the reason *allocation moved* (a vertical took a
//!   payment off an invoice it credits and issues again, such as a lease
//!   amendment) → the payment's own allocation rows to that invoice are
//!   released, so the money is the party's credit again and can be allocated to
//!   the new invoice ([`release_moved`]). A reopening for another reason (a
//!   reversed payment) already removed them.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::drawings::PURPOSE;
use wl_kernel::invoicing::{InvoiceReopened, ReopenReason};
use wl_kernel::payments::{Direction, PAYMENT_SUBJECT_TYPE, PaymentReceived, PaymentReversed};

use crate::allocation::Allocation;
use crate::payment::Payment;
use crate::record::{RECORD, RECORD_BY_SYSTEM, RECORD_SET_OFF, RECORD_SET_OFF_APPROVED};
use crate::reverse::{self, REVERSE, REVERSE_APPROVED, REVERSE_BY_SYSTEM};

/// The event a committed transition of `payment` (as it is now) gives, if
/// any.
fn received_event(transition: &str, payment: &Payment) -> Option<PaymentReceived> {
    // Money out (a refund's voucher) is not a payment received; an owner
    // drawing is (`record` gives `PaymentReceived`, wl-payments §4).
    ([
        RECORD,
        RECORD_BY_SYSTEM,
        RECORD_SET_OFF,
        RECORD_SET_OFF_APPROVED,
    ]
    .contains(&transition)
        && (payment.direction == Direction::In || payment.purpose == PURPOSE))
        .then(|| PaymentReceived {
            payment: payment.payment_id(),
            party: payment.party(),
            amount: payment.amount,
            method: payment.method,
            allocations: Vec::new(),
        })
}

/// The event of a committed reversal of `payment` (as it is now), with the
/// `allocations` it has: the removed ones are listed, with their ids.
/// `None` when the payment carries no reversal.
fn reversed_event(payment: &Payment, allocations: &[Allocation]) -> Option<PaymentReversed> {
    Some(PaymentReversed {
        payment: payment.payment_id(),
        party: payment.party(),
        legal_entity_id: payment.legal_entity_id,
        reason: payment.reversal_reason?,
        source: payment.source,
        statement_line: payment.statement_line_id,
        removed: allocations
            .iter()
            .filter(|a| !a.is_active())
            .map(Allocation::removed)
            .collect(),
        prepaid: payment.prepaid(),
        reversed_on: payment.reversed_on?,
    })
}

/// Releases the payment's allocation to the invoice of `event`, for the amount
/// the invoice gave back to its balance, when the reason is *allocation moved*:
/// the oldest rows first, the rows marked removed (never deleted); a row larger
/// than what is left to release is removed and a new active row keeps the rest.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn release_moved(state: &AppState, event: &InvoiceReopened) -> Result {
    if event.reason != ReopenReason::AllocationMoved
        || event.money_source.subject_type != PAYMENT_SUBJECT_TYPE
    {
        return Ok(());
    }
    let target = event.invoice.subject();
    let rows = Allocation::query()
        .where_(Allocation::PAYMENT_ID.eq(event.money_source.subject_id))
        .where_(Allocation::TARGET_TYPE.eq(target.subject_type.as_str()))
        .where_(Allocation::TARGET_ID.eq(target.subject_id))
        .where_(Allocation::REMOVED_AT.is_null())
        .order_by(Allocation::ID)
        .get(&state.db)
        .await?;
    let now = renox::db::now();
    let mut left = event.amount;
    for row in rows {
        if left <= 0 {
            break;
        }
        let take = left.min(row.amount);
        Allocation::query()
            .where_(Allocation::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("removed_at", &Some(now)),
                    ("removed_reason", &Some("allocation_moved".to_owned())),
                    ("updated_at", &now),
                ],
            )
            .await?;
        if row.amount > take {
            let mut rest = Allocation::new(
                row.payment_id,
                &row.target(),
                row.amount - take,
                row.allocated_by,
            );
            rest.allocated_at = row.allocated_at;
            Allocation::create(&state.db, rest).await?;
        }
        left -= take;
    }
    Ok(())
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: InvoiceReopened, state: AppState| async move {
        release_moved(&state, &event).await
    });
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        let name = event.transition.as_str();
        let recorded = [
            RECORD,
            RECORD_BY_SYSTEM,
            RECORD_SET_OFF,
            RECORD_SET_OFF_APPROVED,
        ]
        .contains(&name);
        let reversed = [REVERSE, REVERSE_BY_SYSTEM, REVERSE_APPROVED].contains(&name);
        if !recorded && !reversed {
            return Ok(());
        }
        // The committed row.
        let payment = Payment::find_or_404(&state.db, event.key).await?;
        if reversed {
            let allocations = Allocation::all_of(&state.db, payment.id).await?;
            if let Some(event) = reversed_event(&payment, &allocations) {
                state.emit(event).await?;
                reverse::after_reversal(&state, &payment).await?;
            }
            return Ok(());
        }
        if let Some(received) = received_event(name, &payment) {
            state.emit(received).await?;
        }
        // An exception for a blocked party was approved: the set-off is
        // allocated to its invoice and its supplier bill now (a set-off a
        // user records is allocated by `set_off::record`).
        if name == RECORD_SET_OFF_APPROVED {
            crate::set_off::complete(&state, payment.id).await?;
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyId;
    use wl_kernel::Subject;
    use wl_kernel::payments::{PaymentId, PaymentMethod, PaymentSource, ReversalReason};

    fn payment() -> Payment {
        Payment {
            id: 4,
            party_id: 7,
            amount: 300_000,
            method: PaymentMethod::Transfer,
            ..Default::default()
        }
    }

    #[test]
    fn record_gives_payment_received_with_no_allocations() {
        for name in [RECORD, RECORD_BY_SYSTEM] {
            let received = received_event(name, &payment()).expect("an event");
            assert_eq!(received.payment, PaymentId(4));
            assert_eq!(received.party, PartyId(7));
            assert_eq!(received.amount, 300_000);
            assert_eq!(received.method, PaymentMethod::Transfer);
            assert!(received.allocations.is_empty());
        }
        assert!(received_event("reverse", &payment()).is_none());
    }

    #[test]
    fn money_out_is_not_a_payment_received() {
        let out = Payment {
            direction: Direction::Out,
            ..payment()
        };
        assert!(received_event(RECORD_BY_SYSTEM, &out).is_none());
    }

    fn reversed_payment() -> Payment {
        Payment {
            reversal_reason: Some(ReversalReason::BankReturn),
            reversed_on: renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 4),
            legal_entity_id: 2,
            source: PaymentSource::BankStatement,
            statement_line_id: Some(77),
            prepaid_type: Some("property.deposit".into()),
            prepaid_id: Some(3),
            ..payment()
        }
    }

    fn allocation(id: i64, invoice: i64, amount: i64, removed: bool) -> Allocation {
        let mut a = Allocation::new(
            4,
            &wl_kernel::invoicing::InvoiceId(invoice).subject(),
            amount,
            None,
        );
        a.id = id;
        if removed {
            a.removed_at = Some(renox::db::now());
        }
        a
    }

    #[test]
    fn reversal_gives_payment_reversed_with_the_removed_allocations() {
        let allocations = [
            allocation(1, 10, 200_000, true),
            allocation(2, 11, 100_000, true),
            allocation(3, 12, 5_000, false),
        ];
        let event = reversed_event(&reversed_payment(), &allocations).expect("an event");
        assert_eq!(event.payment, PaymentId(4));
        assert_eq!(event.party, PartyId(7));
        assert_eq!(event.legal_entity_id, 2);
        assert_eq!(event.reason, ReversalReason::BankReturn);
        let listed: Vec<_> = event
            .removed
            .iter()
            .map(|r| (r.allocation, r.amount))
            .collect();
        assert_eq!(listed, [(1, 200_000), (2, 100_000)]);
        assert_eq!(event.reversed_on.to_string(), "2026-11-04");
    }

    #[test]
    fn the_event_carries_the_source_the_statement_line_and_the_prepaid_record() {
        let event = reversed_event(&reversed_payment(), &[]).expect("an event");
        assert_eq!(event.source, PaymentSource::BankStatement);
        assert_eq!(event.statement_line, Some(77));
        assert_eq!(event.prepaid, Some(Subject::new("property.deposit", 3)));
        assert!(event.removed.is_empty());
    }

    #[test]
    fn a_payment_without_a_reversal_gives_no_event() {
        assert!(reversed_event(&payment(), &[]).is_none());
    }
}
