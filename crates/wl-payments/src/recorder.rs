//! The module's [`PaymentRecorder`] (wl-payments §6): how other modules
//! record, allocate and read payments through the kernel.
//! [`crate::WlPayments`] provides it with
//! [`wl_kernel::payments::provide_payment_recorder`].
//!
//! The kernel passes the acting user as an id (`None` = the system), not as
//! an `AuthUser`. A record therefore runs the system transition
//! `record_by_system` and stores the id in `recorded_by`; the calling module
//! has authorized its user already.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::payments::{
    AllocationTarget, BankPayment, PaymentFuture, PaymentId, PaymentRecorder, PaymentSummary, RecordPayment,
    RefundId, RefundRequest as KernelRefundRequest, ReversalReason, UnpaidRefund,
};

use crate::payment::Payment;
use crate::refund::{self, RefundRequest};
use crate::{allocate, credit, record, reverse};

/// Records, allocates and reads `wl_payments_payments` for the kernel
/// contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Payments;

impl PaymentRecorder for Payments {
    fn record<'a>(
        &'a self,
        state: &'a AppState,
        payment: RecordPayment,
        by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async move {
            let payment = record::record_for(state, by, &payment)
                .await
                .map_err(|e| e.into_error())?;
            credit::summary(&state.db, &payment).await
        })
    }

    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        target: AllocationTarget,
        amount: i64,
    ) -> PaymentFuture<'a, ()> {
        // The contract names no user.
        Box::pin(async move {
            allocate::allocate(state, payment, target, amount, None)
                .await
                .map(|_| ())
        })
    }

    fn reverse<'a>(
        &'a self,
        state: &'a AppState,
        payment: PaymentId,
        reason: ReversalReason,
        note: Option<String>,
        by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        // The contract names a user id, not an `AuthUser`: it runs
        // `reverse_by_system` and stores the id in `reversed_by`.
        Box::pin(async move {
            reverse::reverse_by_id(state, payment, reason, note, by)
                .await
                .map(|_| ())
                .map_err(|e| e.into_error())
        })
    }

    fn find<'a>(&'a self, db: &'a Db, id: PaymentId) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async move {
            match Payment::find(db, id.0).await? {
                Some(payment) => Ok(Some(credit::summary(db, &payment).await?)),
                None => Ok(None),
            }
        })
    }

    fn credit_balance<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(credit::credit_balance(db, party, currency))
    }

    fn payments_for<'a>(
        &'a self,
        db: &'a Db,
        prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(credit::payments_for(db, prepaid))
    }

    fn request_refund<'a>(
        &'a self,
        state: &'a AppState,
        request: KernelRefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        // The caller authorized its user; the refund still needs an
        // approval by another user (`FIN.ACL.04`).
        Box::pin(async move {
            let request = RefundRequest {
                source: request.source,
                party: request.party,
                legal_entity_id: request.legal_entity_id,
                amount: request.amount,
                currency: request.currency,
                method: request.method,
                gateway_txn: request.gateway_txn,
                reason: request.reason,
            };
            refund::request_for_source_as(state, &request, by)
                .await
                .map(|refund| RefundId(refund.id))
                .map_err(|e| e.into_error())
        })
    }

    fn payments_after<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
        bank_account: &'a str,
        after: NaiveDate,
    ) -> PaymentFuture<'a, Vec<BankPayment>> {
        Box::pin(credit::payments_after(db, legal_entity_id, bank_account, after))
    }

    fn refunds_unpaid<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
    ) -> PaymentFuture<'a, Vec<UnpaidRefund>> {
        Box::pin(refund::unpaid(db, legal_entity_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recorder_is_a_payment_recorder() {
        fn takes(_: impl PaymentRecorder) {}
        takes(Payments);
    }
}
