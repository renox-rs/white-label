//! The kernel events the Payment machine emits (wl-payments §6), from a
//! listener on [`Transitioned<Payment>`], so only after a commit and never
//! on a rollback:
//!
//! - `record` / `record_by_system` → [`PaymentReceived`] (a new payment has
//!   no allocations yet).
//!
//! `PaymentReversed` joins with `reverse`.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::payments::{Direction, PaymentReceived};

use crate::payment::Payment;
use crate::record::{RECORD, RECORD_BY_SYSTEM};

/// The event a committed transition of `payment` (as it is now) gives, if
/// any.
fn received_event(transition: &str, payment: &Payment) -> Option<PaymentReceived> {
    // Money out (a refund's voucher) is not a payment received.
    ([RECORD, RECORD_BY_SYSTEM].contains(&transition) && payment.direction == Direction::In).then(
        || PaymentReceived {
            payment: payment.payment_id(),
            party: payment.party(),
            amount: payment.amount,
            method: payment.method,
            allocations: Vec::new(),
        },
    )
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Payment>, state: AppState| async move {
        if ![RECORD, RECORD_BY_SYSTEM].contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed row.
        let payment = Payment::find_or_404(&state.db, event.key).await?;
        match received_event(&event.transition, &payment) {
            Some(received) => state.emit(received).await,
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyId;
    use wl_kernel::payments::{PaymentId, PaymentMethod};

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
}
