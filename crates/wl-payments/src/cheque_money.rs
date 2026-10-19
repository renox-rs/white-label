//! The money a cheque moves (wl-payments §4 *Cheque*: `clear`, `bounce`,
//! `settle_part`): the payment recorded and its allocation to the documents
//! the cheque pays.
//!
//! The payment is recorded in the transition's transaction by the system
//! transition `record_by_system` with the user's id in `recorded_by` (the
//! cheque permissions are not `payments.record`); [`payment_for`] builds the
//! request and [`record_in`] records it. The allocation follows the commit
//! ([`allocate_to_links`]): the invoices and supplier bills the cheque pays,
//! oldest first, each up to its open balance. A link the module cannot pay (a
//! charge not yet invoiced) is skipped, and what is left unallocated stays on
//! the payment as the party's credit for the accountant to allocate.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Outcome;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{PayBill, invoicer, supplier_bills};
use wl_kernel::payments::{
    AllocationTarget, Direction, PaymentMethod, PaymentSource, RecordPayment,
};

use crate::allocate;
use crate::allocation::Allocation;
use crate::cheque::Cheque;
use crate::payment::Payment;
use crate::record::{self, RecordInput};

/// The payment of `amount` fils for `cheque` by `method`, dated today
/// (Asia/Dubai): money in for a received cheque, out for an issued one. A
/// cheque payment carries the cheque's number, bank and date; the others the
/// `reference`.
pub(crate) fn payment_for(
    cheque: &Cheque,
    amount: i64,
    method: PaymentMethod,
    reference: Option<&str>,
) -> RecordPayment {
    let mut request = RecordPayment::new(cheque.party(), cheque.legal_entity_id, method, amount)
        .currency(&cheque.currency)
        .date(today())
        .source(PaymentSource::Cheque);
    if cheque.direction == Direction::Out {
        request = request.outgoing();
    }
    if method == PaymentMethod::Cheque {
        request = request.method_details(json!({
            "number": cheque.number,
            "bank": cheque.bank,
            "date": cheque.cheque_date.format("%Y-%m-%d").to_string(),
        }));
    }
    if let Some(reference) = reference.map(str::trim).filter(|r| !r.is_empty()) {
        request = request.reference(reference);
    }
    if let Some(user) = cheque.collected_by {
        request = request.collected_by(user);
    }
    if let Some(account) = cheque.bank_account.as_deref() {
        request = request.bank_account(account);
    }
    request
}

/// What recording the payment needs, read before the transaction opens.
pub(crate) struct Prepared {
    payment: Payment,
    input: RecordInput,
}

/// The unsaved payment for `request`, recorded by user `by`, and what its
/// guards need.
///
/// # Errors
///
/// `Guard` for a currency that is not an ISO code.
pub(crate) async fn prepare(
    state: &AppState,
    request: &RecordPayment,
    by: Option<i64>,
) -> std::result::Result<Prepared, renox::state_machine::TransitionError> {
    let (payment, input) = record::prepare(state, request, by).await?;
    Ok(Prepared { payment, input })
}

/// Records `prepared` in `tx` (the system transition: the cheque's own
/// permission covers the user). The caller commits, then emits the outcome.
pub(crate) async fn record_in(
    tx: &mut renox::db::Transaction,
    prepared: Prepared,
) -> std::result::Result<(Payment, Outcome<Payment>), renox::state_machine::TransitionError> {
    record::record_in(tx, None, prepared.payment, &prepared.input).await
}

/// Allocates `amount` fils of `payment` to the documents `cheque` pays,
/// oldest first, by user `by`: an invoice up to its open balance (money in),
/// a supplier bill up to what it still owes (money out). Returns how much
/// was allocated. A document that refuses is skipped.
pub(crate) async fn allocate_to_links(
    state: &AppState,
    cheque: &Cheque,
    payment: &Payment,
    amount: i64,
    by: Option<i64>,
) -> Result<i64> {
    let mut left = amount;
    for link in cheque.links(&state.db).await? {
        if left <= 0 {
            break;
        }
        let subject = link.subject();
        match AllocationTarget::from_subject(&subject) {
            Some(AllocationTarget::Invoice(id)) if payment.direction == Direction::In => {
                let Some(invoice) = invoicer(state).find(&state.db, id).await? else {
                    continue;
                };
                let part = left.min(invoice.balance);
                if part <= 0 {
                    continue;
                }
                let target = AllocationTarget::Invoice(id);
                if allocate::allocate(state, payment.payment_id(), target, part, by)
                    .await
                    .is_ok()
                {
                    left -= part;
                }
            }
            Some(AllocationTarget::SupplierBill(id)) if payment.direction == Direction::Out => {
                // The bill's own balance is its module's: ask for what is
                // left, and a bill that owes less refuses.
                let key = format!("payments.cheque:{}:{}", cheque.id, payment.id);
                let request = PayBill::new(id, payment.subject(), left, key);
                if supplier_bills(state).pay(state, request).await.is_ok() {
                    let mut row = Allocation::new(payment.id, &subject, left, by);
                    row.insert(&state.db).await?;
                    left = 0;
                }
            }
            _ => {}
        }
    }
    Ok(amount - left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn cheque() -> Cheque {
        Cheque {
            id: 5,
            party_id: 7,
            legal_entity_id: 2,
            bank: "ENBD".into(),
            number: "1042".into(),
            cheque_date: NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            amount: 2_000_000,
            currency: "AED".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_received_cheques_payment_is_money_in_by_cheque_with_its_details() {
        let request = payment_for(&cheque(), 600_000, PaymentMethod::Cheque, None);
        assert_eq!(request.direction, Direction::In);
        assert_eq!(request.method, PaymentMethod::Cheque);
        assert_eq!(request.amount, 600_000);
        assert_eq!(request.source, PaymentSource::Cheque);
        assert_eq!(request.method_details["number"], "1042");
        assert_eq!(request.method_details["bank"], "ENBD");
        assert_eq!(request.method_details["date"], "2026-11-03");
        assert_eq!(request.date, Some(today()));
    }

    #[test]
    fn an_issued_cheques_payment_is_money_out() {
        let mut issued = cheque();
        issued.direction = Direction::Out;
        let request = payment_for(&issued, 100, PaymentMethod::Cheque, None);
        assert_eq!(request.direction, Direction::Out);
    }

    #[test]
    fn a_part_settlement_carries_its_reference_and_no_cheque_details() {
        let request = payment_for(&cheque(), 400_000, PaymentMethod::Transfer, Some(" TRX-9 "));
        assert_eq!(request.reference.as_deref(), Some("TRX-9"));
        assert!(request.method_details.get("number").is_none());
        let blank = payment_for(&cheque(), 400_000, PaymentMethod::Cash, Some("  "));
        assert_eq!(blank.reference, None);
    }

    #[test]
    fn the_collector_is_kept_on_the_payment() {
        let mut collected = cheque();
        collected.collected_by = Some(9);
        let request = payment_for(&collected, 100, PaymentMethod::Cash, None);
        assert_eq!(request.collected_by, Some(9));
    }
}
