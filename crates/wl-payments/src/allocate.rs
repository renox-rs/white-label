//! Allocating a payment to a target (wl-payments §4, A6).
//!
//! Allocation is not a transition. [`allocate`] checks the payment and the
//! target, calls the target's own module through the kernel
//! (`Invoicer::allocate`, which runs the invoice's `allocate` transition)
//! and, only when that succeeds, writes the [`Allocation`] row. When the
//! invoice refuses, nothing is written and its message is returned.

use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceSummary, invoicer};
use wl_kernel::payments::{AllocationTarget, Direction, PaymentId};

use crate::allocation::Allocation;
use crate::credit;
use crate::payment::Payment;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Why `payment` cannot be allocated `amount` fils when `unallocated` is
/// free, if it cannot.
pub(crate) fn payment_problem(payment: &Payment, unallocated: i64, amount: i64) -> Option<String> {
    if !payment.status.is_received() {
        return Some("Only a received payment can be allocated.".to_owned());
    }
    if payment.direction != Direction::In {
        return Some("Only an incoming payment can be allocated to an invoice.".to_owned());
    }
    if amount <= 0 {
        return Some("The amount to allocate must be above 0.".to_owned());
    }
    if amount > unallocated {
        return Some(format!(
            "The amount is above what is left of the payment ({unallocated} fils)."
        ));
    }
    None
}

/// Why `invoice` cannot take money of `payment`, if it cannot: another
/// party or another currency.
pub(crate) fn invoice_problem(payment: &Payment, invoice: &InvoiceSummary) -> Option<String> {
    if invoice.party != payment.party() {
        return Some("The invoice belongs to another party.".to_owned());
    }
    if !invoice.currency.eq_ignore_ascii_case(&payment.currency) {
        return Some(format!(
            "The invoice is in {}, the payment in {}.",
            invoice.currency, payment.currency
        ));
    }
    None
}

/// Allocates `amount` fils of payment `payment` to `target`, by user `by`
/// (`None` = the system or unknown), and returns the new row.
///
/// # Errors
///
/// 404 for an unknown payment or invoice. 422 for a payment that is not
/// `received` or not incoming, an amount of 0 or less or above the
/// unallocated amount, an invoice of another party or currency, or an
/// invoice that refuses the allocation (nothing is written then).
pub async fn allocate(
    state: &AppState,
    payment: PaymentId,
    target: AllocationTarget,
    amount: i64,
    by: Option<i64>,
) -> Result<Allocation> {
    let payment = Payment::find_or_404(&state.db, payment.0).await?;
    let unallocated = credit::unallocated(&state.db, &payment).await?;
    if let Some(problem) = payment_problem(&payment, unallocated, amount) {
        return Err(refused(problem));
    }
    match target {
        AllocationTarget::Invoice(id) => {
            let invoicer = invoicer(state);
            let invoice = invoicer.find(&state.db, id).await?.ok_or(Error::NotFound)?;
            if let Some(problem) = invoice_problem(&payment, &invoice) {
                return Err(refused(problem));
            }
            // The invoice's own transition; it may refuse (balance, state).
            invoicer
                .allocate(state, id, payment.subject(), amount)
                .await?;
        }
    }
    let mut allocation = Allocation::new(payment.id, &target.subject(), amount, by);
    allocation.insert(&state.db).await?;
    Ok(allocation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::PartyId;
    use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};
    use wl_kernel::payments::PaymentStatus;

    fn payment() -> Payment {
        Payment {
            id: 1,
            party_id: 7,
            amount: 300_000,
            currency: "AED".into(),
            status: PaymentStatus::Received,
            ..Default::default()
        }
    }

    fn invoice(party: i64, currency: &str) -> InvoiceSummary {
        InvoiceSummary {
            id: InvoiceId(4),
            number: Some("INV-1".into()),
            status: InvoiceStatus::Issued,
            party: PartyId(party),
            legal_entity_id: Some(2),
            currency: currency.into(),
            gross_total: 200_000,
            balance: 200_000,
            due_date: None,
        }
    }

    #[test]
    fn a_received_incoming_payment_can_be_allocated_up_to_what_is_left() {
        let p = payment();
        assert_eq!(payment_problem(&p, 300_000, 200_000), None);
        assert_eq!(payment_problem(&p, 300_000, 300_000), None);
        assert!(payment_problem(&p, 100_000, 100_001).is_some());
    }

    #[test]
    fn an_amount_of_zero_or_less_is_refused() {
        let p = payment();
        assert!(payment_problem(&p, 300_000, 0).is_some());
        assert!(payment_problem(&p, 300_000, -5).is_some());
    }

    #[test]
    fn only_a_received_payment_is_allocated() {
        let mut p = payment();
        for status in [
            PaymentStatus::Pending,
            PaymentStatus::Failed,
            PaymentStatus::ChargedBack,
            PaymentStatus::Reversed,
        ] {
            p.status = status;
            let message = payment_problem(&p, 300_000, 100).expect("refused");
            assert!(message.contains("received"), "{message}");
        }
    }

    #[test]
    fn an_outgoing_payment_is_not_allocated_to_an_invoice() {
        let mut p = payment();
        p.direction = Direction::Out;
        assert!(payment_problem(&p, 300_000, 100).is_some());
    }

    #[test]
    fn the_invoice_must_have_the_same_party_and_currency() {
        let p = payment();
        assert_eq!(invoice_problem(&p, &invoice(7, "AED")), None);
        assert_eq!(invoice_problem(&p, &invoice(7, "aed")), None);
        let other_party = invoice_problem(&p, &invoice(8, "AED")).expect("refused");
        assert!(other_party.contains("another party"), "{other_party}");
        let other_currency = invoice_problem(&p, &invoice(7, "USD")).expect("refused");
        assert!(other_currency.contains("USD"), "{other_currency}");
    }
}
