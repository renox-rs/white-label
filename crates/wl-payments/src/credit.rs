//! What is left of a payment and what a party holds as credit (wl-payments
//! §3, §4).
//!
//! A payment's unallocated amount is its amount less its active
//! allocations, and counts only while it is `received`. A party's credit is
//! the sum of those amounts over its incoming `received` payments in a
//! currency. Paid refunds are subtracted from it once the Refund base
//! exists (part 5 of the module).

use std::collections::HashMap;

use renox::prelude::*;
use wl_kernel::payments::{Direction, PaymentStatus, PaymentSummary};
use wl_kernel::{PartyId, Subject};

use crate::allocation::Allocation;
use crate::payment::Payment;

/// The sum of the active allocations of payment `payment_id`, in fils.
pub async fn allocated(db: &Db, payment_id: i64) -> Result<i64> {
    Allocation::query()
        .where_(Allocation::PAYMENT_ID.eq(payment_id))
        .where_(Allocation::REMOVED_AT.is_null())
        .sum::<i64, _>(db, "amount")
        .await
}

/// The active allocation sums of `payments`, by payment id.
async fn allocated_by_payment(db: &Db, payments: &[Payment]) -> Result<HashMap<i64, i64>> {
    let mut sums = HashMap::new();
    if payments.is_empty() {
        return Ok(sums);
    }
    let rows = Allocation::query()
        .where_(Allocation::PAYMENT_ID.is_in(payments.iter().map(|p| p.id)))
        .where_(Allocation::REMOVED_AT.is_null())
        .get(db)
        .await?;
    for row in rows {
        *sums.entry(row.payment_id).or_insert(0) += row.amount;
    }
    Ok(sums)
}

/// What is still free on `payment`, in fils: 0 unless it is `received`.
pub async fn unallocated(db: &Db, payment: &Payment) -> Result<i64> {
    Ok(payment
        .summary(allocated(db, payment.id).await?)
        .unallocated)
}

/// The summary of `payment`, with its allocated and unallocated amounts.
pub async fn summary(db: &Db, payment: &Payment) -> Result<PaymentSummary> {
    Ok(payment.summary(allocated(db, payment.id).await?))
}

/// The party's credit in `currency`, in fils: the unallocated amounts of its
/// incoming `received` payments.
pub async fn credit_balance(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
    let payments = Payment::query()
        .where_(Payment::PARTY_ID.eq(party.0))
        .where_(Payment::CURRENCY.eq(currency.trim().to_ascii_uppercase()))
        .where_(Payment::DIRECTION.eq(Direction::In))
        .where_(Payment::STATUS.eq(PaymentStatus::Received))
        .get(db)
        .await?;
    let sums = allocated_by_payment(db, &payments).await?;
    Ok(payments
        .iter()
        .map(|p| p.summary(sums.get(&p.id).copied().unwrap_or(0)).unallocated)
        .sum())
}

/// The payments made in advance for `prepaid`, oldest first.
pub async fn payments_for(db: &Db, prepaid: &Subject) -> Result<Vec<PaymentSummary>> {
    let payments = Payment::query()
        .where_(Payment::PREPAID_TYPE.eq(prepaid.subject_type.as_str()))
        .where_(Payment::PREPAID_ID.eq(prepaid.subject_id))
        .order_by(Payment::PAYMENT_DATE)
        .order_by(Payment::ID)
        .get(db)
        .await?;
    let sums = allocated_by_payment(db, &payments).await?;
    Ok(payments
        .iter()
        .map(|p| p.summary(sums.get(&p.id).copied().unwrap_or(0)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payment(id: i64, amount: i64, status: PaymentStatus) -> Payment {
        Payment {
            id,
            amount,
            status,
            ..Default::default()
        }
    }

    #[test]
    fn the_unallocated_amount_is_what_the_allocations_leave() {
        let p = payment(1, 300_000, PaymentStatus::Received);
        assert_eq!(p.summary(0).unallocated, 300_000);
        assert_eq!(p.summary(200_000).unallocated, 100_000);
        assert_eq!(p.summary(300_000).unallocated, 0);
        // Never below 0, even for a bad row.
        assert_eq!(p.summary(400_000).unallocated, 0);
    }

    #[test]
    fn only_a_received_payment_has_credit() {
        for status in [
            PaymentStatus::Pending,
            PaymentStatus::Failed,
            PaymentStatus::ChargedBack,
            PaymentStatus::Reversed,
        ] {
            assert_eq!(payment(1, 100, status).summary(0).unallocated, 0);
        }
    }
}
