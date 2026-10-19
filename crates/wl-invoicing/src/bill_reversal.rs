//! The consumer of `PaymentReversed` for supplier bills (wl-invoicing §4
//! `unallocate`, #75 A7).
//!
//! For every removed allocation whose target is a supplier bill, [`handle`]
//! runs the bill's system `unallocate` with the payment as source, the
//! removed amount, the key `payments.allocation:<id>`, the reason
//! `payment_reversed` and the payment's own reason (`bank_return`, ...).
//! Invoice targets stay with [`crate::reversal`]; the two listeners filter by
//! target type. The key makes a repeated delivery change nothing.
//!
//! One failing allocation does not stop the others. A refusal (logged
//! `failed` by the run) opens a follow-up task for the accountant
//! ([`crate::task`]); only an unexpected error is returned, after the rest
//! ran. A cheque cancelled before it clears sends no event, so its bill is
//! not touched.

use renox::prelude::*;
use wl_kernel::invoicing::{
    BillReopenReason, SUPPLIER_BILL_SUBJECT_TYPE, SupplierBillId, UnallocateBill,
};
use wl_kernel::payments::{PaymentReversed, RemovedAllocation};

use crate::bill_unallocate;
use crate::reversal::{key_of, reversal_key};
use crate::supplier_bill::SupplierBill;
use crate::task;

/// The unallocations `event` asks for: one per removed allocation whose
/// target is a supplier bill, in the event's order.
pub fn requests_for(event: &PaymentReversed) -> Vec<UnallocateBill> {
    let reason = reversal_key(event.reason);
    event
        .removed
        .iter()
        .filter(|removed| is_bill(removed))
        .map(|removed| {
            UnallocateBill::new(
                SupplierBillId(removed.target.subject_id),
                event.payment.subject(),
                removed.amount,
                key_of(removed.allocation),
                BillReopenReason::PaymentReversed,
            )
            .reversal_reason(reason.as_str())
        })
        .collect()
}

/// Whether the allocation was to a supplier bill.
fn is_bill(removed: &RemovedAllocation) -> bool {
    removed.target.subject_type == SUPPLIER_BILL_SUBJECT_TYPE
}

/// Runs `request` and, when it is refused, opens the accountant's task. A
/// refusal is an answer, not a failure: `Ok`. Any other error is returned.
///
/// # Errors
///
/// An error that is not a refusal of the request.
pub async fn run_with_task(state: &AppState, request: UnallocateBill) -> Result {
    let error = match bill_unallocate::run(state, request.clone()).await {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    let reason = match &error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => message.clone(),
        // Logged `failed` by the run; no bill to point a task at.
        Error::NotFound => return Ok(()),
        _ => return Err(error),
    };
    let Some(bill) = SupplierBill::find(&state.db, request.bill.0).await? else {
        return Ok(());
    };
    task::open_for_bill(
        state,
        &bill,
        &request.source,
        request.amount,
        &request.key,
        &reason,
    )
    .await?;
    Ok(())
}

/// Unallocates every supplier bill allocation `event` removed.
///
/// # Errors
///
/// The first unexpected error, after every allocation was tried.
pub async fn handle(state: &AppState, event: &PaymentReversed) -> Result {
    let mut first_error = None;
    for request in requests_for(event) {
        if let Err(error) = run_with_task(state, request).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Registers the listener on `PaymentReversed`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: PaymentReversed, state: AppState| async move { handle(&state, &event).await },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;
    use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
    use wl_kernel::payments::{PaymentId, PaymentSource, ReversalReason};
    use wl_kernel::{PartyId, Subject};

    fn event(removed: Vec<RemovedAllocation>) -> PaymentReversed {
        PaymentReversed {
            payment: PaymentId(3),
            party: PartyId(7),
            legal_entity_id: 1,
            reason: ReversalReason::BankReturn,
            source: PaymentSource::Manual,
            statement_line: None,
            removed,
            prepaid: None,
            reversed_on: NaiveDate::from_ymd_opt(2026, 12, 10).unwrap(),
        }
    }

    fn removed(allocation: i64, subject_type: &str, id: i64, amount: i64) -> RemovedAllocation {
        RemovedAllocation {
            allocation,
            target: Subject::new(subject_type, id),
            amount,
        }
    }

    #[test]
    fn one_request_per_bill_allocation() {
        let requests = requests_for(&event(vec![
            removed(11, SUPPLIER_BILL_SUBJECT_TYPE, 4, 1_050_000),
            removed(12, INVOICE_SUBJECT_TYPE, 2, 5_000),
            removed(13, SUPPLIER_BILL_SUBJECT_TYPE, 5, 80_000),
        ]));
        assert_eq!(requests.len(), 2);
        let first = &requests[0];
        assert_eq!(first.bill, SupplierBillId(4));
        assert_eq!(first.source, Subject::new("payments.payment", 3));
        assert_eq!(first.amount, 1_050_000);
        assert_eq!(first.key, "payments.allocation:11");
        assert_eq!(first.reason, BillReopenReason::PaymentReversed);
        assert_eq!(first.reversal_reason.as_deref(), Some("bank_return"));
        assert_eq!(requests[1].key, "payments.allocation:13");
        assert_eq!(requests[1].bill, SupplierBillId(5));
    }

    #[test]
    fn invoice_targets_and_empty_reversals_ask_for_nothing() {
        assert!(requests_for(&event(vec![])).is_empty());
        assert!(requests_for(&event(vec![removed(1, INVOICE_SUBJECT_TYPE, 2, 1)])).is_empty());
    }
}
