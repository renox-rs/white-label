//! A payments recorder for the delivery tests: the crate does not depend on
//! `wl-payments`, so the tests provide a `PaymentRecorder` that only records
//! the refunds the module asks for (everything else is refused, as with the
//! payments module off).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::payments::{
    AllocationTarget, PaymentFuture, PaymentId, PaymentRecorder, PaymentSummary, RecordPayment,
    RefundId, RefundRequest, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

/// The recording `PaymentRecorder`.
#[derive(Clone, Default)]
pub struct FakePayments(Arc<Mutex<Vec<(RefundRequest, Option<i64>)>>>);

fn off() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "payments is not enabled".to_owned(),
    )
}

impl FakePayments {
    /// The refunds asked for, with the user who asked, oldest first.
    pub fn refunds(&self) -> Vec<(RefundRequest, Option<i64>)> {
        self.0.lock().unwrap().clone()
    }
}

impl PaymentRecorder for FakePayments {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async { Err(off()) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: PaymentId,
    ) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async { Ok(None) })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        _prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        request: RefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async move {
            let mut refunds = self.0.lock().unwrap();
            refunds.push((request, by));
            Ok(RefundId(i64::try_from(refunds.len()).unwrap_or(i64::MAX)))
        })
    }
}
