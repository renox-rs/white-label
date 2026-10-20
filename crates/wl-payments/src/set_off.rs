//! Setting off a party's invoice against its supplier bill (wl-payments §4
//! *Set-off*, `PTY.ACL.03`).
//!
//! When a party is both our customer and our supplier and the two sides
//! agree to settle against each other, the accountant records one payment of
//! method `set_off`. It moves no money and has no bank account: one record
//! with the amount, allocated at once to the party's open invoice (the
//! invoice runs `allocate`) and to its approved supplier bill (the bill runs
//! `pay`), of the same entity and currency, for the same amount, not above
//! either open balance. It is an incoming payment of the party (a receipt,
//! numbered as one), shown in receipts and payments with the method
//! *set-off* and never matched with a bank statement.
//!
//! [`record`] reads the invoice and the bill through the kernel contracts,
//! checks them with [`problem`], and asks the block guard: because it
//! settles a payable, a set-off is money out to the party
//! (`ActionKind::MoneyOut`). For a blocked party no payment is recorded: a
//! `pending` payment holds the request, and an exception request goes to
//! the approver the guard names (the compliance role). At the final
//! approval the kernel runs `record_set_off_approved` (the same guards and
//! effect, run by the system), whose listener ([`complete`]) allocates.
//!
//! The payment is recorded first and allocated after its commit, because
//! the invoice and the bill are other modules' records changed through
//! their own contracts. When either side refuses, the payment is reversed
//! at once as *entered in error*, and the reversal's event gives back
//! whatever was allocated, so a set-off is never settled on one side only.
//!
//! The set-off is corrected only with `reverse` (reason *entered in error*):
//! it removes both allocations, and `PaymentReversed` makes the invoice and
//! the supplier bill run `unallocate`.
//!
//! A bill that is `partially_paid` can be set off for what it still owes,
//! like an `approved` one (both take `pay`); a draft, matched, held or
//! voided bill, and a paid one, are refused.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::invoicing::{
    InvoiceId, InvoiceStatus, InvoiceSummary, PayBill, SupplierBillId, SupplierBillStatus,
    SupplierBillSummary, invoicer, supplier_bills,
};
use wl_kernel::payments::{
    AllocationTarget, PaymentId, PaymentMethod, PaymentSource, RecordPayment, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

use crate::allocation::Allocation;
use crate::config_of;
use crate::payment::Payment;
use crate::permissions;
use crate::record::{RECORD_SET_OFF, RecordInput, prepare};
use crate::reverse::reverse_by_id;

/// The code of the method in the settings: `set_off`.
pub const METHOD_CODE: &str = "set_off";

/// The invoice states that take money.
const OPEN_INVOICE: [InvoiceStatus; 3] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
];

/// What the accountant gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetOffRequest {
    /// The party that is both customer and supplier.
    pub party: PartyId,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The party's open invoice (the receivable).
    pub invoice: InvoiceId,
    /// The party's approved supplier bill (the payable).
    pub bill: SupplierBillId,
    /// The amount, in fils of the shared currency.
    pub amount: i64,
    /// The day of the set-off; today when empty.
    pub date: Option<NaiveDate>,
    /// A reference; the bill's number when empty.
    pub reference: Option<String>,
}

/// What [`record`] did.
#[derive(Debug, Clone)]
pub enum SetOffOutcome {
    /// The set-off was recorded and allocated to both documents.
    Recorded(Payment),
    /// The party is blocked (`PTY.ACL.03`): nothing was recorded or
    /// allocated. A `pending` payment holds the request and an exception
    /// was requested from the approver the block guard names.
    ExceptionRequested(ApprovalRequest),
}

/// Why the set-off cannot be recorded for `invoice` and `bill`, if it cannot.
pub fn problem(
    request: &SetOffRequest,
    invoice: &InvoiceSummary,
    bill: &SupplierBillSummary,
) -> Option<String> {
    if request.amount <= 0 {
        return Some("The amount must be above 0.".to_owned());
    }
    if invoice.party != request.party {
        return Some("The invoice belongs to another party.".to_owned());
    }
    if bill.supplier != request.party {
        return Some("The supplier bill is from another party.".to_owned());
    }
    if invoice.legal_entity_id != Some(request.legal_entity_id)
        || bill.legal_entity_id != Some(request.legal_entity_id)
    {
        return Some("The invoice and the supplier bill are of different entities.".to_owned());
    }
    if !invoice.currency.eq_ignore_ascii_case(&bill.currency) {
        return Some(format!(
            "The invoice is in {}, the supplier bill in {}.",
            invoice.currency, bill.currency
        ));
    }
    if !OPEN_INVOICE.contains(&invoice.status) {
        return Some("The invoice is not open for payment.".to_owned());
    }
    if !matches!(
        bill.status,
        SupplierBillStatus::Approved | SupplierBillStatus::PartiallyPaid
    ) {
        return Some(
            "Only an approved supplier bill can be set off (a bill on hold must be approved first)."
                .to_owned(),
        );
    }
    if request.amount > invoice.balance {
        return Some(format!(
            "The amount is above the invoice's open balance ({} fils).",
            invoice.balance
        ));
    }
    if request.amount > bill.balance {
        return Some(format!(
            "The amount is above the supplier bill's open balance ({} fils).",
            bill.balance
        ));
    }
    None
}

/// The invoice and the supplier bill a set-off `payment` names in its method
/// details, if it names both.
pub fn targets_of(payment: &Payment) -> Option<(InvoiceId, SupplierBillId)> {
    let id = |key: &str| payment.method_details.0.get(key).and_then(Value::as_i64);
    Some((InvoiceId(id("invoice")?), SupplierBillId(id("bill")?)))
}

/// The payment request of `request`: money in by set-off, with no bank
/// account, in the documents' currency.
fn payment_request(
    request: &SetOffRequest,
    invoice: &InvoiceSummary,
    bill: &SupplierBillSummary,
    by: i64,
) -> RecordPayment {
    let reference = request
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map_or_else(|| format!("Set-off against {}", bill.number), str::to_owned);
    let mut payment = RecordPayment::new(
        request.party,
        request.legal_entity_id,
        PaymentMethod::SetOff,
        request.amount,
    )
    .currency(&invoice.currency)
    .reference(&reference)
    .source(PaymentSource::Manual)
    .collected_by(by)
    .method_details(json!({
        "invoice": request.invoice.0,
        "bill": request.bill.0,
    }));
    if let Some(date) = request.date {
        payment = payment.date(date);
    }
    payment
}

/// Whether money may not go out to `party` now (`PTY.ACL.03`): asked as the
/// system, so a restricted reason never shows.
async fn is_blocked(state: &AppState, party: PartyId) -> Result<bool> {
    let mut tx = state.db.begin().await?;
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, None, party, ActionKind::MoneyOut)
        .await?;
    // Only a question: nothing was written.
    drop(tx);
    Ok(!matches!(outcome, GuardOutcome::Allowed))
}

/// A 422 with `message`.
fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// Records the set-off of `request` as `user`. See the module docs.
///
/// # Errors
///
/// `Forbidden` without `payments.record`; 404 for an unknown invoice or
/// bill; `Guard` (422) when the method `set_off` is not enabled, or
/// [`problem`] says so; nothing is written then. A blocked party is not an
/// error: it gives [`SetOffOutcome::ExceptionRequested`].
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    request: &SetOffRequest,
) -> std::result::Result<SetOffOutcome, TransitionError> {
    if !user.has_permission_anywhere(permissions::RECORD) {
        return Err(TransitionError::Forbidden);
    }
    if !config_of(state).method_enabled(METHOD_CODE) {
        return Err(guard("The payment method set_off is not enabled."));
    }
    let invoice = invoicer(state)
        .find(&state.db, request.invoice)
        .await?
        .ok_or(Error::NotFound)?;
    let bill = supplier_bills(state)
        .find(&state.db, request.bill)
        .await?
        .ok_or(Error::NotFound)?;
    if let Some(message) = problem(request, &invoice, &bill) {
        return Err(guard(message));
    }
    let payment_request = payment_request(request, &invoice, &bill, user.id);
    let (payment, mut input) = prepare(state, &payment_request, Some(user.id)).await?;
    // The method is enabled: checked above.
    input.method_enabled = true;
    if is_blocked(state, request.party).await? {
        return ask_exception(state, user, payment, &input).await;
    }
    let mut tx = state.db.begin().await?;
    let mut payment = payment;
    payment.insert(&mut tx).await?;
    let outcome = payment
        .transition(RECORD_SET_OFF)
        .by(user)
        .input(&input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    // `Transitioned<Payment>` -> `PaymentReceived` (crate::events).
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    complete(state, payment.id).await?;
    Ok(SetOffOutcome::Recorded(
        Payment::find_or_404(&state.db, payment.id).await?,
    ))
}

/// The party is blocked: the `pending` payment is kept as the request's
/// subject and the exception goes to the approver. The payload is what the
/// approved run reads as its input.
async fn ask_exception(
    state: &AppState,
    user: &AuthUser,
    mut payment: Payment,
    input: &RecordInput,
) -> std::result::Result<SetOffOutcome, TransitionError> {
    let payload = renox::serde_json::to_value(input)
        .map_err(|error| TransitionError::Failed(Error::from(renox::anyhow::anyhow!(error))))?;
    let mut tx = state.db.begin().await?;
    payment.insert(&mut tx).await?;
    let request = block_guard::request_exception_in(
        state,
        &mut tx,
        user,
        &payment.subject(),
        RECORD_SET_OFF,
        payload,
    )
    .await?;
    tx.commit().await?;
    approvals::announce_request(state, &request).await?;
    Ok(SetOffOutcome::ExceptionRequested(request))
}

/// Allocates the recorded set-off `payment_id` to its invoice and its
/// supplier bill, once. When either side refuses, the payment is reversed as
/// *entered in error* (the event gives back what was allocated) and the
/// refusal is returned.
///
/// # Errors
///
/// The refusal of the invoice or the bill, or a database error.
pub(crate) async fn complete(state: &AppState, payment_id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, payment_id).await?;
    if !payment.status.is_received() || payment.method != PaymentMethod::SetOff {
        return Ok(());
    }
    let Some((invoice, bill)) = targets_of(&payment) else {
        return Ok(());
    };
    // Allocated before: a repeated call changes nothing.
    if Allocation::all_of(&state.db, payment.id)
        .await?
        .iter()
        .any(Allocation::is_active)
    {
        return Ok(());
    }
    let by = payment.recorded_by;
    match allocate_both(state, &payment, invoice, bill, by).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let note = "The set-off could not be allocated to both documents.";
            if let Err(failed) = reverse_by_id(
                state,
                PaymentId(payment.id),
                ReversalReason::EnteredInError,
                Some(note.to_owned()),
                by,
            )
            .await
            {
                return Err(failed.into_error());
            }
            Err(error)
        }
    }
}

/// The two allocations: the invoice's `allocate`, then the bill's `pay`.
async fn allocate_both(
    state: &AppState,
    payment: &Payment,
    invoice: InvoiceId,
    bill: SupplierBillId,
    by: Option<i64>,
) -> Result {
    crate::allocate::allocate(
        state,
        payment.payment_id(),
        AllocationTarget::Invoice(invoice),
        payment.amount,
        by,
    )
    .await?;
    let key = format!("payments.set_off:{}", payment.id);
    supplier_bills(state)
        .pay(
            state,
            PayBill::new(bill, payment.subject(), payment.amount, key),
        )
        .await?;
    let target: Subject = bill.subject();
    let mut row = Allocation::new(payment.id, &target, payment.amount, by);
    row.insert(&state.db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::db::Json;

    const PARTY: PartyId = PartyId(7);

    fn request(amount: i64) -> SetOffRequest {
        SetOffRequest {
            party: PARTY,
            legal_entity_id: 2,
            invoice: InvoiceId(4),
            bill: SupplierBillId(9),
            amount,
            date: None,
            reference: None,
        }
    }

    fn invoice(balance: i64) -> InvoiceSummary {
        InvoiceSummary {
            id: InvoiceId(4),
            number: Some("INV-0410".into()),
            status: InvoiceStatus::Issued,
            party: PARTY,
            legal_entity_id: Some(2),
            currency: "AED".into(),
            gross_total: 10_500_000,
            balance,
            due_date: None,
            disputed: false,
            chasing_paused_until: None,
        }
    }

    fn bill(balance: i64) -> SupplierBillSummary {
        SupplierBillSummary {
            id: SupplierBillId(9),
            number: "SB-77".into(),
            supplier: PARTY,
            legal_entity_id: Some(2),
            currency: "AED".into(),
            status: SupplierBillStatus::Approved,
            balance,
        }
    }

    #[test]
    fn an_amount_within_both_balances_is_accepted() {
        let open = invoice(10_500_000);
        assert_eq!(problem(&request(525_000), &open, &bill(525_000)), None);
        // The rest of either stays open.
        assert_eq!(problem(&request(100), &open, &bill(525_000)), None);
    }

    #[test]
    fn an_amount_above_either_balance_or_not_above_zero_is_refused() {
        let open = invoice(10_500_000);
        let above_bill = problem(&request(600_000), &open, &bill(525_000)).expect("refused");
        assert!(above_bill.contains("supplier bill's open balance"), "{above_bill}");
        let above_invoice = problem(&request(600_000), &invoice(500_000), &bill(900_000))
            .expect("refused");
        assert!(above_invoice.contains("invoice's open balance"), "{above_invoice}");
        for amount in [0, -5] {
            assert!(problem(&request(amount), &open, &bill(525_000)).is_some());
        }
    }

    #[test]
    fn the_documents_share_the_party_the_entity_and_the_currency() {
        let open = invoice(10_500_000);
        let mut other = open.clone();
        other.party = PartyId(8);
        assert!(
            problem(&request(100), &other, &bill(525_000))
                .is_some_and(|m| m.contains("another party"))
        );
        let mut foreign = bill(525_000);
        foreign.supplier = PartyId(8);
        assert!(problem(&request(100), &open, &foreign).is_some());
        let mut other_entity = bill(525_000);
        other_entity.legal_entity_id = Some(3);
        assert!(
            problem(&request(100), &open, &other_entity)
                .is_some_and(|m| m.contains("different entities"))
        );
        let mut usd = bill(525_000);
        usd.currency = "USD".into();
        assert!(
            problem(&request(100), &open, &usd).is_some_and(|m| m.contains("USD"))
        );
        let mut lower = bill(525_000);
        lower.currency = "aed".into();
        assert_eq!(problem(&request(100), &open, &lower), None);
    }

    #[test]
    fn only_an_open_invoice_and_an_approved_bill_are_set_off() {
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Paid,
            InvoiceStatus::WrittenOff,
            InvoiceStatus::Credited,
        ] {
            let mut closed = invoice(10_500_000);
            closed.status = status;
            assert!(problem(&request(100), &closed, &bill(525_000)).is_some(), "{status:?}");
        }
        for status in [
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Overdue,
        ] {
            let mut open = invoice(10_500_000);
            open.status = status;
            assert_eq!(problem(&request(100), &open, &bill(525_000)), None, "{status:?}");
        }
        for status in [
            SupplierBillStatus::Draft,
            SupplierBillStatus::Matched,
            SupplierBillStatus::Exception,
            SupplierBillStatus::Paid,
            SupplierBillStatus::Void,
        ] {
            let mut held = bill(525_000);
            held.status = status;
            assert!(
                problem(&request(100), &invoice(10_500_000), &held).is_some(),
                "{status:?}"
            );
        }
        let mut part = bill(525_000);
        part.status = SupplierBillStatus::PartiallyPaid;
        assert_eq!(problem(&request(100), &invoice(10_500_000), &part), None);
    }

    #[test]
    fn the_payment_names_both_documents_and_has_no_bank_account() {
        let built = payment_request(&request(525_000), &invoice(10_500_000), &bill(525_000), 3);
        assert_eq!(built.method, PaymentMethod::SetOff);
        assert_eq!(built.bank_account, None);
        assert_eq!(built.currency, "AED");
        assert_eq!(built.reference.as_deref(), Some("Set-off against SB-77"));
        assert_eq!(built.method_details["invoice"], 4);
        assert_eq!(built.method_details["bill"], 9);
        let payment = Payment {
            method_details: Json(built.method_details),
            ..Default::default()
        };
        assert_eq!(targets_of(&payment), Some((InvoiceId(4), SupplierBillId(9))));
        assert_eq!(targets_of(&Payment::default()), None);
        let named = SetOffRequest {
            reference: Some(" Agreed 3 Nov ".into()),
            ..request(100)
        };
        let built = payment_request(&named, &invoice(1), &bill(1), 3);
        assert_eq!(built.reference.as_deref(), Some("Agreed 3 Nov"));
    }
}
