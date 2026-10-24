//! Paying approved supplier bills (wl-payments §4 *Payment* `record`, §5.1;
//! wl-invoicing §5.4 point 4; `PTY.ACL.03`; BL-PAY-28).
//!
//! The accountant pays one supplier by bank transfer against its `approved`
//! and `partially_paid` bills ([`payable_bills`], read through the kernel's
//! bill search, oldest first; a bill held as a possible duplicate is not
//! approved and is not offered). One outgoing payment (a payment voucher
//! number) carries the amount per bill: each amount at most the bill's open
//! balance, all bills of one receiving entity, and their total is the
//! payment (no unallocated money out). [`propose`] spreads an amount over the
//! oldest bills first.
//!
//! The payment is recorded with `record_supplier` and then each bill runs its
//! `pay` ([`complete`]); when a bill refuses, the payment is reversed at once
//! as *entered in error* so no bill is left half paid. A reversal later
//! (*entered in error*) removes the allocations and the bills run
//! `unallocate` (wl-invoicing's consumer of `PaymentReversed`).
//!
//! **Money out to a blocked supplier** is not recorded: [`pay`] gives
//! [`SupplierPayOutcome::OnHold`] with the on-hold text, and
//! [`request_exception`] keeps a `pending` payment with the bills as the
//! request's subject and asks the compliance role (BL-PTY-04). At the final
//! approval the kernel re-checks the guards and runs
//! `record_supplier_approved` as the system with the stored payload: the
//! payment is recorded once, as the requester, with the stored amounts, and
//! allocated by the listener. A supplier unblocked meanwhile is paid at once
//! by [`request_exception`] without a request. A changed payload needs a new
//! request (the kernel's rule).
//!
//! **An issued cheque** is not a payment yet: [`pay`] registers an issued
//! cheque linked to the bills, which pays them at `clear` as before; the
//! block guard runs when it is issued (a blocked supplier gets the on-hold
//! refusal).
//!
//! `FIN.ACL.21` (an approved request before money out) is for owner drawings
//! only: an ordinary supplier payment needs no approval of its own, the bills
//! were approved.

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::invoice_search::invoice_search;
use wl_kernel::invoicing::{PayBill, SupplierBillId, SupplierBillSummary, supplier_bills};
use wl_kernel::payments::{
    Direction, PaymentId, PaymentMethod, PaymentSource, RecordPayment, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

use crate::allocation::Allocation;
use crate::cheque::Cheque;
use crate::payment::Payment;
use crate::permissions;
use crate::record::{RECORD_SUPPLIER, RecordInput, prepare};
use crate::reverse::reverse_by_id;

/// What the accountant gives to pay a supplier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplierPayment {
    /// The supplier.
    pub supplier: PartyId,
    /// The receiving entity whose bills are paid (and whose account pays).
    pub legal_entity_id: i64,
    /// `transfer` or `cheque` (an issued cheque).
    pub method: PaymentMethod,
    /// The bank account the money leaves from.
    pub bank_account: Option<String>,
    /// The day; today when empty.
    pub date: Option<NaiveDate>,
    /// The bank's reference of the transfer, or the cheque number.
    pub reference: Option<String>,
    /// The bank of an issued cheque.
    pub cheque_bank: Option<String>,
    /// The amount per bill.
    pub bills: Vec<(SupplierBillId, i64)>,
}

impl SupplierPayment {
    /// The total of the bills.
    pub fn total(&self) -> i64 {
        self.bills.iter().map(|(_, amount)| amount).sum()
    }
}

/// What [`pay`] did.
#[derive(Debug, Clone)]
pub enum SupplierPayOutcome {
    /// The transfer is recorded and each bill paid.
    Paid(Box<Payment>),
    /// An issued cheque is registered; it pays the bills at `clear`.
    ChequeIssued(Box<Cheque>),
    /// The supplier is blocked: nothing was recorded. The text is the
    /// neutral on-hold text; the accountant may request an exception.
    OnHold(String),
}

/// What [`request_exception`] did.
#[derive(Debug, Clone)]
pub enum ExceptionOutcome {
    /// The supplier is blocked: the request waits for the compliance role.
    Requested(Box<ApprovalRequest>),
    /// The supplier is no longer blocked: paid at once.
    Paid(Box<Payment>),
}

/// The bills of `supplier` that take a payment, oldest first.
pub async fn payable_bills(
    state: &AppState,
    supplier: PartyId,
) -> Result<Vec<SupplierBillSummary>> {
    invoice_search(state)
        .payable_bills_of(&state.db, supplier)
        .await
}

/// `amount` spread over `bills` (oldest first): each bill up to its balance
/// until the amount is used.
pub fn propose(bills: &[SupplierBillSummary], amount: i64) -> Vec<(SupplierBillId, i64)> {
    let mut left = amount;
    let mut out = Vec::new();
    for bill in bills {
        if left <= 0 {
            break;
        }
        let part = left.min(bill.balance);
        if part > 0 {
            out.push((bill.id, part));
            left -= part;
        }
    }
    out
}

/// Why `request` cannot be paid against `bills` (the supplier's payable
/// bills), if it cannot.
pub fn problem(request: &SupplierPayment, bills: &[SupplierBillSummary]) -> Option<String> {
    if request.bills.is_empty() {
        return Some("Choose the bills to pay.".to_owned());
    }
    if !matches!(
        request.method,
        PaymentMethod::Transfer | PaymentMethod::Cheque
    ) {
        return Some("A supplier is paid by transfer or by an issued cheque.".to_owned());
    }
    let mut seen = BTreeSet::new();
    for (id, amount) in &request.bills {
        if !seen.insert(id.0) {
            return Some("A bill is named twice.".to_owned());
        }
        let Some(bill) = bills.iter().find(|b| b.id == *id) else {
            return Some(format!(
                "Bill {} is not an approved bill of this supplier.",
                id.0
            ));
        };
        if bill.legal_entity_id != Some(request.legal_entity_id) {
            return Some("One payment pays the bills of one entity only.".to_owned());
        }
        if *amount <= 0 {
            return Some("Each amount must be above 0.".to_owned());
        }
        if *amount > bill.balance {
            return Some(format!(
                "The amount for bill {} is above its open balance.",
                bill.number
            ));
        }
    }
    None
}

/// The bills a supplier payment names in its method details, as
/// `(bill, amount)`.
pub fn bills_of(payment: &Payment) -> Vec<(SupplierBillId, i64)> {
    payment
        .method_details
        .0
        .get("bills")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let pair = row.as_array()?;
                    Some((
                        SupplierBillId(pair.first()?.as_i64()?),
                        pair.get(1)?.as_i64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The on-hold text when money may not go out to `party` now, asked as the
/// system so a restricted reason never shows.
async fn on_hold(state: &AppState, party: PartyId) -> Result<Option<String>> {
    let mut tx = state.db.begin().await?;
    let outcome = block_guard::guard(state)
        .check_in(&mut tx, None, party, ActionKind::MoneyOut)
        .await?;
    drop(tx);
    Ok(match outcome {
        GuardOutcome::Allowed => None,
        GuardOutcome::Blocked(refusal) => Some(refusal.message_en()),
    })
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The outgoing transfer of `request`, by `by`.
fn payment_request(request: &SupplierPayment, currency: &str, by: i64) -> RecordPayment {
    let reference = request
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or_default()
        .to_owned();
    let bills: Vec<Value> = request
        .bills
        .iter()
        .map(|(id, amount)| json!([id.0, amount]))
        .collect();
    let mut payment = RecordPayment::new(
        request.supplier,
        request.legal_entity_id,
        PaymentMethod::Transfer,
        request.total(),
    )
    .currency(currency)
    .reference(&reference)
    .source(PaymentSource::Manual)
    .collected_by(by)
    .method_details(json!({ "reference": reference, "bills": bills }));
    payment.direction = Direction::Out;
    payment.bank_account = request.bank_account.clone();
    if let Some(date) = request.date {
        payment = payment.date(date);
    }
    payment
}

/// The checks every path shares: the permission, the supplier's bills.
async fn checked(
    state: &AppState,
    user: &AuthUser,
    request: &SupplierPayment,
) -> std::result::Result<Vec<SupplierBillSummary>, TransitionError> {
    if !user.has_permission_anywhere(permissions::RECORD) {
        return Err(TransitionError::Forbidden);
    }
    let bills = payable_bills(state, request.supplier).await?;
    if let Some(message) = problem(request, &bills) {
        return Err(guard(message));
    }
    Ok(bills)
}

/// `user` pays `request`. See the module docs.
///
/// # Errors
///
/// `Forbidden` without `payments.record`; `Guard` (422) when [`problem`]
/// says so or the payment is refused (a transfer without a reference, a
/// disabled method). A blocked supplier is not an error: it gives
/// [`SupplierPayOutcome::OnHold`].
pub async fn pay(
    state: &AppState,
    user: &AuthUser,
    request: &SupplierPayment,
) -> std::result::Result<SupplierPayOutcome, TransitionError> {
    let bills = checked(state, user, request).await?;
    if let Some(text) = on_hold(state, request.supplier).await? {
        return Ok(SupplierPayOutcome::OnHold(text));
    }
    if request.method == PaymentMethod::Cheque {
        return issue_cheque(state, user, request, &bills).await;
    }
    let currency = currency_of(&bills);
    let payment_request = payment_request(request, &currency, user.id);
    let (payment, input) = prepare(state, &payment_request, Some(user.id)).await?;
    let payment = record_now(state, user, payment, &input).await?;
    Ok(SupplierPayOutcome::Paid(Box::new(payment)))
}

/// The currency of the bills (they are one entity's), AED when none.
fn currency_of(bills: &[SupplierBillSummary]) -> String {
    bills
        .first()
        .map_or_else(|| "AED".to_owned(), |b| b.currency.clone())
}

/// Records `payment` with `record_supplier` as `user`, then pays its bills.
async fn record_now(
    state: &AppState,
    user: &AuthUser,
    mut payment: Payment,
    input: &RecordInput,
) -> std::result::Result<Payment, TransitionError> {
    let mut tx = state.db.begin().await?;
    payment.insert(&mut tx).await?;
    let outcome = payment
        .transition(RECORD_SUPPLIER)
        .by(user)
        .input(input)
        .run_in(&mut tx)
        .await?;
    tx.commit().await?;
    // `Transitioned<Payment>` -> `PaymentReceived` (crate::events).
    outcome.emit(state).await.map_err(TransitionError::Failed)?;
    complete(state, payment.id).await?;
    Ok(Payment::find_or_404(&state.db, payment.id).await?)
}

/// Registers the issued cheque of `request`, linked to its bills.
async fn issue_cheque(
    state: &AppState,
    user: &AuthUser,
    request: &SupplierPayment,
    bills: &[SupplierBillSummary],
) -> std::result::Result<SupplierPayOutcome, TransitionError> {
    let number = request
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| guard("An issued cheque needs its number."))?;
    let bank = request
        .cheque_bank
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| guard("An issued cheque needs its bank."))?;
    let mut register = RegisterCheque::new(
        request.supplier,
        request.legal_entity_id,
        bank,
        number,
        request.date.unwrap_or_else(wl_kernel::documents::today),
        request.total(),
    );
    register.direction = Direction::Out;
    register.currency = currency_of(bills);
    register.bank_account = request.bank_account.clone();
    register.linked = request.bills.iter().map(|(id, _)| id.subject()).collect();
    let summary = cheques(state)
        .register(state, register, Some(user.id))
        .await?;
    let cheque = Cheque::find_or_404(&state.db, summary.id.0).await?;
    Ok(SupplierPayOutcome::ChequeIssued(Box::new(cheque)))
}

/// `user` asks for an exception to pay the blocked supplier of `request` by
/// transfer: a `pending` payment holds the request and its amounts. A
/// supplier no longer blocked is paid at once.
///
/// # Errors
///
/// As [`pay`]; 422 for an issued cheque (it is issued once the supplier is
/// unblocked or the exception is given on the transfer).
pub async fn request_exception(
    state: &AppState,
    user: &AuthUser,
    request: &SupplierPayment,
) -> std::result::Result<ExceptionOutcome, TransitionError> {
    let bills = checked(state, user, request).await?;
    if request.method != PaymentMethod::Transfer {
        return Err(guard("An exception is asked for a transfer."));
    }
    let currency = currency_of(&bills);
    let payment_request = payment_request(request, &currency, user.id);
    let (mut payment, input) = prepare(state, &payment_request, Some(user.id)).await?;
    if on_hold(state, request.supplier).await?.is_none() {
        let paid = record_now(state, user, payment, &input).await?;
        return Ok(ExceptionOutcome::Paid(Box::new(paid)));
    }
    let payload = renox::serde_json::to_value(&input)
        .map_err(|error| TransitionError::Failed(Error::from(renox::anyhow::anyhow!(error))))?;
    let mut tx = state.db.begin().await?;
    payment.insert(&mut tx).await?;
    let asked = block_guard::request_exception_in(
        state,
        &mut tx,
        user,
        &payment.subject(),
        RECORD_SUPPLIER,
        payload,
    )
    .await?;
    tx.commit().await?;
    approvals::announce_request(state, &asked).await?;
    Ok(ExceptionOutcome::Requested(Box::new(asked)))
}

/// Pays the bills of the recorded supplier payment `payment_id`, once (each
/// bill's `pay`, then the allocation row). When a bill refuses, the payment
/// is reversed as *entered in error* (its event gives back what was paid)
/// and the refusal is returned.
///
/// # Errors
///
/// The refusal of a bill, or a database error.
pub(crate) async fn complete(state: &AppState, payment_id: i64) -> Result {
    let payment = Payment::find_or_404(&state.db, payment_id).await?;
    if !payment.status.is_received() || payment.direction != Direction::Out {
        return Ok(());
    }
    let bills = bills_of(&payment);
    if bills.is_empty() {
        return Ok(());
    }
    // Paid before: a repeated call changes nothing.
    if Allocation::all_of(&state.db, payment.id)
        .await?
        .iter()
        .any(Allocation::is_active)
    {
        return Ok(());
    }
    let by = payment.recorded_by;
    for (bill, amount) in &bills {
        let key = format!("payments.supplier:{}:{}", payment.id, bill.0);
        let paid = supplier_bills(state)
            .pay(state, PayBill::new(*bill, payment.subject(), *amount, key))
            .await;
        if let Err(error) = paid {
            let note = "A bill refused the supplier payment.";
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
            return Err(error);
        }
        let target: Subject = bill.subject();
        let mut row = Allocation::new(payment.id, &target, *amount, by);
        row.insert(&state.db).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::SupplierBillStatus;

    fn bill(id: i64, number: &str, balance: i64) -> SupplierBillSummary {
        SupplierBillSummary {
            id: SupplierBillId(id),
            number: number.into(),
            supplier: PartyId(7),
            legal_entity_id: Some(2),
            currency: "AED".into(),
            status: SupplierBillStatus::Approved,
            balance,
        }
    }

    fn request(bills: Vec<(i64, i64)>) -> SupplierPayment {
        SupplierPayment {
            supplier: PartyId(7),
            legal_entity_id: 2,
            method: PaymentMethod::Transfer,
            bank_account: None,
            date: None,
            reference: Some("TRF-1".into()),
            cheque_bank: None,
            bills: bills
                .into_iter()
                .map(|(id, amount)| (SupplierBillId(id), amount))
                .collect(),
        }
    }

    #[test]
    fn an_amount_is_spread_over_the_oldest_bills_first() {
        let open = [bill(1, "B-101", 105_000), bill(2, "B-102", 210_000)];
        assert_eq!(
            propose(&open, 150_000),
            vec![(SupplierBillId(1), 105_000), (SupplierBillId(2), 45_000)]
        );
        assert_eq!(
            propose(&open, 315_000),
            vec![(SupplierBillId(1), 105_000), (SupplierBillId(2), 210_000)]
        );
    }

    #[test]
    fn an_amount_above_a_bill_balance_is_refused() {
        let open = [bill(1, "B-101", 105_000)];
        assert!(problem(&request(vec![(1, 105_000)]), &open).is_none());
        let message = problem(&request(vec![(1, 105_001)]), &open).unwrap();
        assert!(message.contains("above its open balance"), "{message}");
        assert!(problem(&request(vec![(9, 100)]), &open).is_some());
        assert!(problem(&request(vec![]), &open).is_some());
    }
}
