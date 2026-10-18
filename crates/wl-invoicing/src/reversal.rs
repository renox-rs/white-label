//! The consumer of `PaymentReversed` (wl-invoicing §4 `unallocate`, #70 A6).
//!
//! wl-payments removes the allocations of a reversed payment and emits
//! `PaymentReversed` with each one (id, target, amount). For every removed
//! allocation whose target is an invoice, [`handle`] runs the system
//! `unallocate` with the payment as source, the removed amount, the key
//! `payments.allocation:<id>`, the reason `payment_reversed` and the
//! payment's own reason (`bank_return`, ...). The key makes a repeated
//! delivery change nothing.
//!
//! One failing allocation does not stop the others. A refusal (the run is
//! logged `failed`) opens a follow-up task for the accountant
//! ([`crate::task`]); only an unexpected error (the database) is returned,
//! after the rest ran, so the event can be delivered again. With wl-payments
//! off the event never comes and nothing changes.

use renox::prelude::*;
use wl_kernel::invoicing::{INVOICE_SUBJECT_TYPE, InvoiceId, ReopenReason, Unallocate};
use wl_kernel::payments::{PaymentReversed, RemovedAllocation, ReversalReason};

use crate::invoice::Invoice;
use crate::task;
use crate::unallocate;

/// The idempotency key of the unallocation for a removed allocation.
pub fn key_of(allocation: i64) -> String {
    format!("payments.allocation:{allocation}")
}

/// The payment's reversal reason as its key (`bank_return`, ...).
pub fn reversal_key(reason: ReversalReason) -> String {
    match renox::serde_json::to_value(reason) {
        Ok(renox::serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

/// The unallocations `event` asks for: one per removed allocation whose
/// target is an invoice, in the event's order.
pub fn requests_for(event: &PaymentReversed) -> Vec<Unallocate> {
    let reason = reversal_key(event.reason);
    event
        .removed
        .iter()
        .filter(|removed| is_invoice(removed))
        .map(|removed| {
            Unallocate::new(
                InvoiceId(removed.target.subject_id),
                event.payment.subject(),
                removed.amount,
                key_of(removed.allocation),
                ReopenReason::PaymentReversed,
            )
            .reversal_reason(reason.as_str())
        })
        .collect()
}

/// Whether the allocation was to an invoice.
fn is_invoice(removed: &RemovedAllocation) -> bool {
    removed.target.subject_type == INVOICE_SUBJECT_TYPE
}

/// Runs `request` and, when it is refused, opens the accountant's task.
/// A refusal is an answer, not a failure: `Ok`. Any other error is
/// returned.
///
/// # Errors
///
/// An error that is not a refusal of the request.
pub async fn run_with_task(state: &AppState, request: Unallocate) -> Result {
    let error = match unallocate::run(state, request.clone()).await {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    let reason = match &error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message) => message.clone(),
        // Logged `failed` by the run; no invoice to point a task at.
        Error::NotFound => return Ok(()),
        _ => return Err(error),
    };
    let Some(invoice) = Invoice::find(&state.db, request.invoice.0).await? else {
        return Ok(());
    };
    task::open(
        state,
        &invoice,
        &request.source,
        request.amount,
        &request.key,
        &reason,
    )
    .await?;
    Ok(())
}

/// Unallocates every invoice allocation `event` removed (see the module).
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
    use wl_kernel::payments::{PaymentId, PaymentSource};
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
    fn one_request_per_invoice_allocation() {
        let requests = requests_for(&event(vec![
            removed(11, INVOICE_SUBJECT_TYPE, 4, 120_000),
            removed(12, "property.deposit", 2, 5_000),
            removed(13, INVOICE_SUBJECT_TYPE, 5, 80_000),
        ]));
        assert_eq!(requests.len(), 2);
        let first = &requests[0];
        assert_eq!(first.invoice, InvoiceId(4));
        assert_eq!(first.source, Subject::new("payments.payment", 3));
        assert_eq!(first.amount, 120_000);
        assert_eq!(first.key, "payments.allocation:11");
        assert_eq!(first.reason, ReopenReason::PaymentReversed);
        assert_eq!(first.reversal_reason.as_deref(), Some("bank_return"));
        assert_eq!(requests[1].key, "payments.allocation:13");
        assert_eq!(requests[1].invoice, InvoiceId(5));
    }

    #[test]
    fn a_reversal_without_invoice_allocations_asks_for_nothing() {
        assert!(requests_for(&event(vec![])).is_empty());
        assert!(requests_for(&event(vec![removed(1, "property.deposit", 2, 1)])).is_empty());
    }

    #[test]
    fn the_reversal_reason_is_its_key() {
        assert_eq!(reversal_key(ReversalReason::BankReturn), "bank_return");
        assert_eq!(
            reversal_key(ReversalReason::EnteredInError),
            "entered_in_error"
        );
        assert_eq!(reversal_key(ReversalReason::Chargeback), "chargeback");
    }
}
