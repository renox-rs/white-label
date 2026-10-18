//! Refund attempts (wl-payments §4, A3): each try to pay a refund.
//!
//! `pay` opens an attempt with its outgoing payment, already `paid`;
//! `send_to_gateway` opens one `pending` with the gateway's refund
//! reference. An attempt that fails keeps its kind and reason and is never
//! deleted or rewritten.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::payments::{PaymentMethod, RefundFailureKind};

/// How an attempt went.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttemptOutcome {
    /// Sent to the gateway; waiting for its answer.
    #[default]
    Pending,
    /// The money went out.
    Paid,
    /// Declined by the gateway or returned by the bank.
    Failed,
}

/// One try to pay a refund.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_refund_attempts",
    unique(refund_id, number),
    unique(gateway_ref),
    index(outcome)
)]
pub struct RefundAttempt {
    /// The primary key.
    pub id: i64,
    /// The refund it tries to pay.
    pub refund_id: i64,
    /// 1, 2, ... per refund.
    pub number: i32,
    /// How it was paid.
    pub method: PaymentMethod,
    /// The payment reference.
    pub reference: Option<String>,
    /// The gateway's refund reference; unique when set.
    pub gateway_ref: Option<String>,
    /// The outgoing payment `pay` recorded.
    pub payment_id: Option<i64>,
    /// The day it started (Asia/Dubai).
    pub started_on: NaiveDate,
    /// How it went.
    pub outcome: AttemptOutcome,
    /// Why it failed.
    pub failure_kind: Option<RefundFailureKind>,
    /// The reason given for the failure.
    pub failure_reason: Option<String>,
    /// The day it failed (Asia/Dubai).
    pub failed_on: Option<NaiveDate>,
    /// The user who failed it; `None` for the system.
    pub failed_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RefundAttempt {
    /// The attempts of `refund_id`, oldest first.
    pub async fn of_refund(db: &Db, refund_id: i64) -> Result<Vec<RefundAttempt>> {
        RefundAttempt::query()
            .where_(RefundAttempt::REFUND_ID.eq(refund_id))
            .order_by(RefundAttempt::NUMBER)
            .get(db)
            .await
    }

    /// The latest attempt of `refund_id`, if any.
    pub async fn latest(db: &Db, refund_id: i64) -> Result<Option<RefundAttempt>> {
        Ok(Self::of_refund(db, refund_id).await?.pop())
    }

    /// The attempt that holds the gateway reference `gateway_ref`.
    pub async fn find_by_gateway_ref(db: &Db, gateway_ref: &str) -> Result<Option<RefundAttempt>> {
        RefundAttempt::query()
            .where_(RefundAttempt::GATEWAY_REF.eq(gateway_ref))
            .first(db)
            .await
    }

    /// The number the next attempt of `refund_id` gets.
    pub(crate) fn next_number(existing: u64) -> i32 {
        i32::try_from(existing)
            .unwrap_or(i32::MAX)
            .saturating_add(1)
    }

    /// Opens the next attempt of `refund_id` in `tx`, started today.
    pub(crate) async fn open_in(
        tx: &mut renox::db::Transaction,
        refund_id: i64,
        method: PaymentMethod,
        reference: Option<String>,
        gateway_ref: Option<String>,
        payment_id: Option<i64>,
        outcome: AttemptOutcome,
    ) -> Result<RefundAttempt> {
        let existing = RefundAttempt::query()
            .where_(RefundAttempt::REFUND_ID.eq(refund_id))
            .count(&mut *tx)
            .await?;
        let mut attempt = RefundAttempt {
            refund_id,
            number: Self::next_number(existing),
            method,
            reference,
            gateway_ref,
            payment_id,
            started_on: today(),
            outcome,
            ..Default::default()
        };
        attempt.insert(&mut *tx).await?;
        Ok(attempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attempt_starts_pending_and_the_outcomes_have_names() {
        assert_eq!(AttemptOutcome::default(), AttemptOutcome::Pending);
        assert_eq!(AttemptOutcome::Failed.as_str(), "failed");
        assert_eq!(AttemptOutcome::Paid.as_str(), "paid");
        assert!(RefundAttempt::default().failure_kind.is_none());
    }

    #[test]
    fn attempts_are_numbered_from_one() {
        assert_eq!(RefundAttempt::next_number(0), 1);
        assert_eq!(RefundAttempt::next_number(2), 3);
    }
}
