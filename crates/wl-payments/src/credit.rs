//! What is left of a payment and what a party holds as credit (wl-payments
//! §3, §4).
//!
//! A payment's unallocated amount is its amount less its active
//! allocations, and counts only while it is `received`. A party's credit is
//! the sum of those amounts over its incoming `received` payments in a
//! currency, less the party's `paid` refunds, so it is below 0 when the
//! credit was refunded and the payment was reversed afterwards. A refund
//! claims its source's balance from `requested` on, so what is
//! *refundable* is what is left after the pending ones.

use std::collections::HashMap;

use renox::prelude::*;
use wl_kernel::payments::{Direction, PaymentStatus, PaymentSummary};
use wl_kernel::{PartyId, Subject};

use crate::allocation::Allocation;
use crate::payment::Payment;
use crate::refund::{Refund, RefundStatus};

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

/// The sum of the refunds of `party` in `currency` in `statuses`, in fils;
/// only those of `source` when given.
async fn refund_sum(
    db: &Db,
    party: PartyId,
    currency: &str,
    statuses: &[RefundStatus],
    source: Option<&Subject>,
) -> Result<i64> {
    let mut query = Refund::query()
        .where_(Refund::PARTY_ID.eq(party.0))
        .where_(Refund::CURRENCY.eq(currency.trim().to_ascii_uppercase()))
        .where_(Refund::STATUS.is_in(statuses.iter().copied()));
    if let Some(source) = source {
        query = query
            .where_(Refund::SOURCE_TYPE.eq(source.subject_type.as_str()))
            .where_(Refund::SOURCE_ID.eq(source.subject_id));
    }
    query.sum::<i64, _>(db, "amount").await
}

/// The party's credit in `currency`, in fils: the unallocated amounts of its
/// incoming `received` payments less its `paid` refunds, plus the
/// deposit-sourced refunds that `failed` or were `cancelled` (the money held
/// as a deposit is back on the party). Below 0 when the credit was refunded
/// and the payment reversed afterwards.
pub async fn credit_balance(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
    let payments = Payment::query()
        .where_(Payment::PARTY_ID.eq(party.0))
        .where_(Payment::CURRENCY.eq(currency.trim().to_ascii_uppercase()))
        .where_(Payment::DIRECTION.eq(Direction::In))
        .where_(Payment::STATUS.eq(PaymentStatus::Received))
        .get(db)
        .await?;
    let sums = allocated_by_payment(db, &payments).await?;
    let unallocated: i64 = payments
        .iter()
        .map(|p| p.summary(sums.get(&p.id).copied().unwrap_or(0)).unallocated)
        .sum();
    let paid = paid_from_credit(db, party, currency).await?;
    let returned = deposit_returned(db, party, currency).await?;
    Ok(unallocated - paid + returned)
}

/// The `paid` refunds of `party` in `currency` out of its payments or its
/// credit, in fils. A refund of a deposit (or of a source another module
/// owns) never counted in the party's credit, so paying it takes nothing
/// from it.
async fn paid_from_credit(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
    Refund::query()
        .where_(Refund::PARTY_ID.eq(party.0))
        .where_(Refund::CURRENCY.eq(currency.trim().to_ascii_uppercase()))
        .where_(Refund::SOURCE_KIND.is_in(["payment", "party_credit"]))
        .where_(Refund::STATUS.eq(RefundStatus::Paid))
        .sum::<i64, _>(db, "amount")
        .await
}

/// The deposit-sourced refunds of `party` in `currency` that `failed` or
/// were `cancelled`, in fils: the deposit's money sits on the party's credit
/// until the refund is retried or paid another way, and is never lost.
async fn deposit_returned(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
    Refund::query()
        .where_(Refund::PARTY_ID.eq(party.0))
        .where_(Refund::CURRENCY.eq(currency.trim().to_ascii_uppercase()))
        .where_(Refund::SOURCE_KIND.eq(DEPOSIT_KIND))
        .where_(Refund::STATUS.is_in(RETURNED_TO_CREDIT))
        .sum::<i64, _>(db, "amount")
        .await
}

/// The `source_kind` of a refund out of a deposit.
const DEPOSIT_KIND: &str = "deposit";

/// The states in which a deposit-sourced refund gives its amount back to the
/// party's credit.
const RETURNED_TO_CREDIT: [RefundStatus; 2] = [RefundStatus::Failed, RefundStatus::Cancelled];

/// What the party can still ask to have refunded out of its credit in
/// `currency`, in fils: the credit balance less the refunds waiting to be
/// paid; never below 0.
pub async fn refundable_credit(db: &Db, party: PartyId, currency: &str) -> Result<i64> {
    let balance = credit_balance(db, party, currency).await?;
    let pending = refund_sum(db, party, currency, &RefundStatus::PENDING, None).await?;
    Ok((balance - pending).max(0))
}

/// What can still be refunded out of `payment`, in fils: its unallocated
/// amount less the refunds that claim it, and no more than the party's
/// refundable credit; never below 0.
pub async fn refundable_payment(db: &Db, payment: &Payment) -> Result<i64> {
    let source = payment.subject();
    let claimed = refund_sum(
        db,
        payment.party(),
        &payment.currency,
        &RefundStatus::ACTIVE,
        Some(&source),
    )
    .await?;
    let own = unallocated(db, payment).await? - claimed;
    let pool = refundable_credit(db, payment.party(), &payment.currency).await?;
    Ok(own.min(pool).max(0))
}

/// Whether the source of `refund` still covers its amount when it is paid:
/// a payment must still be `received` with enough unallocated money left
/// after the refunds already paid out of it, and the party's credit balance
/// (which already counts the paid refunds) must reach the amount.
pub async fn covers(db: &Db, refund: &Refund) -> Result<bool> {
    // Only a refund out of the party's own money is bounded by its credit;
    // a deposit (held apart) or a source another module owns is vouched for
    // by its owner when requested, and never counted in the credit.
    if matches!(refund.source_kind.as_str(), "payment" | "party_credit") {
        let balance = credit_balance(db, refund.party(), &refund.currency).await?;
        if refund.amount > balance {
            return Ok(false);
        }
    }
    let Some(payment_id) = refund.source_payment() else {
        return Ok(true);
    };
    let Some(payment) = Payment::find(db, payment_id.0).await? else {
        return Ok(false);
    };
    let paid = refund_sum(
        db,
        refund.party(),
        &refund.currency,
        &[RefundStatus::Paid],
        Some(&payment.subject()),
    )
    .await?;
    Ok(refund.amount <= unallocated(db, &payment).await? - paid)
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
    fn a_failed_or_cancelled_deposit_refund_gives_its_amount_back() {
        assert!(RETURNED_TO_CREDIT.contains(&RefundStatus::Failed));
        assert!(RETURNED_TO_CREDIT.contains(&RefundStatus::Cancelled));
        for status in [
            RefundStatus::Requested,
            RefundStatus::Approved,
            RefundStatus::Paid,
            RefundStatus::Rejected,
        ] {
            assert!(!RETURNED_TO_CREDIT.contains(&status), "{status:?}");
        }
        // The kind a deposit-sourced refund is stored with.
        let deposit = wl_kernel::payments::DepositId(3);
        assert_eq!(
            wl_kernel::payments::RefundSource::Deposit(deposit).kind(),
            DEPOSIT_KIND
        );
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
