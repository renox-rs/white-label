//! The module's [`PaymentRecorder`] (wl-payments §6): how other modules
//! record, allocate and read payments through the kernel.
//! [`crate::WlPayments`] provides it with
//! [`wl_kernel::payments::provide_payment_recorder`].
//!
//! The kernel passes the acting user as an id (`None` = the system), not as
//! an `AuthUser`. A record therefore runs the system transition
//! `record_by_system` and stores the id in `recorded_by`; the calling module
//! has authorized its user already.

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::payments::{
    AllocationTarget, PaymentFuture, PaymentId, PaymentRecorder, PaymentSummary, RecordPayment,
    ReversalReason,
};

use crate::payment::Payment;
use crate::{allocate, credit, record};

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
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        // The `reverse` transitions come with part 6 of the module.
        Box::pin(async {
            Err(Error::Status(
                StatusCode::NOT_IMPLEMENTED,
                "payment reversal is not available yet".to_owned(),
            ))
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
