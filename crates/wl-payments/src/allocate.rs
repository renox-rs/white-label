//! Allocating a payment to a target (wl-payments §4, A6).
//!
//! Allocation is not a transition. [`allocate`] checks the payment and the
//! target, calls the target's own module through the kernel
//! (`Invoicer::allocate`, which runs the invoice's `allocate` transition)
//! and, only when that succeeds, writes the [`Allocation`] row. When the
//! invoice refuses, nothing is written and its message is returned.
//!
//! A deposit target is this module's own: no `Invoicer` is called. The
//! allocation row is written, and the deposit's held amount rises, in one
//! transaction ([`allocate_to_deposit_in`]).

use renox::prelude::*;
use wl_kernel::invoicing::{InvoiceSummary, invoicer};
use wl_kernel::payments::{AllocationTarget, Direction, PaymentId};

use crate::allocation::Allocation;
use crate::credit;
use crate::deposit::{Deposit, DepositStatus};
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
    if payment.purpose == wl_kernel::drawings::PURPOSE {
        return Some("An owner drawings payment cannot be allocated to a document.".to_owned());
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

/// Why `payment` cannot put `amount` fils into `deposit` when `unallocated`
/// is free on the payment, if it cannot: the payment checks of
/// [`payment_problem`], another party or currency, a deposit that is no
/// longer `held`, or more than the deposit still lacks.
pub(crate) fn deposit_problem(
    payment: &Payment,
    deposit: &Deposit,
    unallocated: i64,
    amount: i64,
) -> Option<String> {
    if let Some(problem) = payment_problem(payment, unallocated, amount) {
        return Some(problem);
    }
    if deposit.party_id != payment.party_id {
        return Some("The deposit belongs to another party.".to_owned());
    }
    if !deposit.currency.eq_ignore_ascii_case(&payment.currency) {
        return Some(format!(
            "The deposit is in {}, the payment in {}.",
            deposit.currency, payment.currency
        ));
    }
    if deposit.status != DepositStatus::Held {
        return Some("Only a held deposit takes money.".to_owned());
    }
    let lacking = deposit.amount - deposit.held_amount;
    if amount > lacking {
        return Some(format!(
            "The amount is above what the deposit still lacks ({lacking} fils)."
        ));
    }
    None
}

/// Allocates `amount` fils of `payment` to `deposit` in `tx` and raises the
/// deposit's held amount (also on `deposit`). The caller checked
/// [`deposit_problem`].
pub(crate) async fn allocate_to_deposit_in(
    tx: &mut renox::db::Transaction,
    payment: &Payment,
    deposit: &mut Deposit,
    amount: i64,
    by: Option<i64>,
) -> Result<Allocation> {
    let mut allocation = Allocation::new(payment.id, &deposit.subject(), amount, by);
    allocation.insert(&mut *tx).await?;
    let held = deposit.held_amount + amount;
    // Not the status column, so outside the machine's guard.
    Deposit::query()
        .where_(Deposit::ID.eq(deposit.id))
        .update(
            &mut *tx,
            &[("held_amount", &held), ("updated_at", &renox::db::now())],
        )
        .await?;
    deposit.held_amount = held;
    // A top-up is paid once the deposit holds its new amount.
    crate::deposit_adjust::settle_top_ups_in(&mut *tx, deposit.id, held, deposit.amount).await?;
    Ok(allocation)
}

/// Allocates `amount` fils of payment `payment` to `target`, by user `by`
/// (`None` = the system or unknown), and returns the new row.
///
/// # Errors
///
/// 404 for an unknown payment, invoice or deposit. 422 for a payment that is not
/// `received` or not incoming, an amount of 0 or less or above the
/// unallocated amount, an invoice of another party or currency, or an
/// invoice that refuses the allocation, or a deposit of another party or
/// currency, no longer `held`, or lacking less than the amount (nothing is
/// written then).
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
        AllocationTarget::SupplierBill(id) => {
            return Err(refused(format!(
                "supplier bill {id} cannot be paid from here yet"
            )));
        }
        AllocationTarget::SupplierDebitNote(id) => {
            return Err(refused(format!(
                "supplier debit note {id} cannot be settled from here yet"
            )));
        }
        AllocationTarget::Deposit(id) => {
            let mut deposit = Deposit::find(&state.db, id.0)
                .await?
                .ok_or(Error::NotFound)?;
            if let Some(problem) = deposit_problem(&payment, &deposit, unallocated, amount) {
                return Err(refused(problem));
            }
            let mut tx = state.db.begin().await?;
            let allocation =
                allocate_to_deposit_in(&mut tx, &payment, &mut deposit, amount, by).await?;
            tx.commit().await?;
            return Ok(allocation);
        }
    }
    let mut allocation = Allocation::new(payment.id, &target.subject(), amount, by);
    allocation.insert(&state.db).await?;
    // The payment takes the scope of the invoice it pays (BL-AGT-04).
    crate::search::index(&state.db, &[payment.subject()]).await?;
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
            disputed: false,
            chasing_paused_until: None,
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

    fn deposit(party: i64, currency: &str, held: i64) -> Deposit {
        Deposit {
            id: 2,
            party_id: party,
            amount: 800_000,
            held_amount: held,
            currency: currency.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_deposit_takes_money_of_its_party_and_currency_up_to_what_it_lacks() {
        let p = payment();
        assert_eq!(
            deposit_problem(&p, &deposit(7, "AED", 0), 300_000, 300_000),
            None
        );
        assert_eq!(
            deposit_problem(&p, &deposit(7, "aed", 500_000), 300_000, 300_000),
            None
        );
        let above =
            deposit_problem(&p, &deposit(7, "AED", 600_000), 300_000, 300_000).expect("refused");
        assert!(above.contains("200000"), "{above}");
        let party = deposit_problem(&p, &deposit(8, "AED", 0), 300_000, 100).expect("refused");
        assert!(party.contains("another party"), "{party}");
        let currency = deposit_problem(&p, &deposit(7, "USD", 0), 300_000, 100).expect("refused");
        assert!(currency.contains("USD"), "{currency}");
    }

    #[test]
    fn only_a_held_deposit_takes_money() {
        let p = payment();
        let mut d = deposit(7, "AED", 0);
        d.status = DepositStatus::SettlementPending;
        let message = deposit_problem(&p, &d, 300_000, 100).expect("refused");
        assert!(message.contains("held"), "{message}");
    }

    #[test]
    fn a_deposit_allocation_keeps_the_deposit_as_its_target() {
        let d = deposit(7, "AED", 0);
        let allocation = Allocation::new(1, &d.subject(), 100, None);
        assert_eq!(
            allocation.allocation_target(),
            Some(AllocationTarget::Deposit(d.deposit_id()))
        );
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
