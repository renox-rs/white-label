//! The fee collected at the front desk (10 §3.1: the counsellor records
//! payments; §4.3 *Fees to cash*).
//!
//! [`record`] takes the money the student or the guardian hands over for one
//! enrolment: it records the payment in wl-payments (cash, card or bank
//! transfer; the receipt number comes from there) and allocates it to the open
//! invoices of the enrolment, the first invoice before the instalments. The
//! invoice of the first payment that is paid in full activates the enrolment as
//! if the payment had come by link (`InvoicePaid`). The counsellor needs no
//! access to the invoices: the enrolment page is the place.

use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{InvoiceId, invoicer};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};

use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentStatus, Instalment};
use crate::permissions;
use crate::texts::english;

/// The methods the desk takes, as the form names them.
pub const METHODS: [&str; 3] = ["cash", "card", "transfer"];

/// The method named `code`.
fn method_of(code: &str) -> Option<PaymentMethod> {
    match code.trim() {
        "cash" => Some(PaymentMethod::Cash),
        "card" => Some(PaymentMethod::Card),
        "transfer" => Some(PaymentMethod::Transfer),
        _ => None,
    }
}

/// The open invoices of `enrolment` with what is left on each, in the order
/// they are paid: the first invoice, then the instalments by number.
///
/// # Errors
///
/// The database's errors and the contract's.
pub async fn open_invoices(state: &AppState, enrolment: &Enrolment) -> Result<Vec<(i64, i64)>> {
    let mut ids: Vec<i64> = enrolment.first_invoice_id.into_iter().collect();
    for row in Instalment::of_enrolment(&state.db, enrolment.id).await? {
        if let Some(id) = row.invoice_id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    let books = invoicer(state);
    let mut open = Vec::new();
    for id in ids {
        if let Some(summary) = books.find(&state.db, InvoiceId(id)).await?
            && summary.balance > 0
        {
            open.push((id, summary.balance));
        }
    }
    Ok(open)
}

/// What the desk may still take for `enrolment`, in fils: the balance of its
/// open invoices.
///
/// # Errors
///
/// The database's errors and the contract's.
pub async fn due(state: &AppState, enrolment: &Enrolment) -> Result<i64> {
    Ok(open_invoices(state, enrolment)
        .await?
        .iter()
        .map(|(_, balance)| balance)
        .sum())
}

/// Records `amount` fils paid by `method` (`cash`, `card` or `transfer`) with
/// `reference` for enrolment `id`, and allocates it to the invoices of the
/// enrolment. Returns the receipt number.
///
/// # Errors
///
/// A 403 without `enrolments.manage` and `payments.record`; a 422 for an
/// enrolment that takes no payment (withdrawn, lapsed, transferred), a method
/// the desk does not take, an amount that is not above 0 or above what is
/// due, a bank transfer without its reference, or a payment wl-payments
/// refuses.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    method: &str,
    amount: i64,
    reference: &str,
) -> Result<String> {
    if !by.has_permission(permissions::ENROLMENTS_MANAGE)
        || !by.has_permission(permissions::PAYMENTS_RECORD)
    {
        return Err(Error::Forbidden);
    }
    let enrolment = Enrolment::find_or_404(&state.db, id).await?;
    if !matches!(
        enrolment.status,
        EnrolmentStatus::PendingPayment | EnrolmentStatus::Active | EnrolmentStatus::Frozen
    ) {
        return Err(refused(english("edtech.desk.problem.closed", &[])));
    }
    let Some(method) = method_of(method) else {
        return Err(refused(english("edtech.desk.problem.method", &[])));
    };
    let open = open_invoices(state, &enrolment).await?;
    let due: i64 = open.iter().map(|(_, balance)| balance).sum();
    if due <= 0 {
        return Err(refused(english("edtech.desk.problem.nothing_due", &[])));
    }
    if amount <= 0 {
        return Err(refused(english("edtech.desk.problem.amount", &[])));
    }
    if amount > due {
        return Err(refused(english(
            "edtech.desk.problem.too_much",
            &[("due", &crate::money_text(due))],
        )));
    }
    let reference = reference.trim();
    if method == PaymentMethod::Transfer && reference.is_empty() {
        return Err(refused(english("edtech.desk.problem.reference", &[])));
    }
    let scope = enrolment.campus_type.as_deref().zip(enrolment.campus_id);
    let entity = LegalEntity::for_scope(&state.db, scope).await?;
    let mut request = RecordPayment::new(enrolment.payer(), entity.id, method, amount);
    request.reference = (!reference.is_empty()).then(|| reference.to_owned());
    request.prepaid = Some(enrolment.subject());
    request.collected_by = Some(by.id);
    let recorder = payment_recorder(state);
    let payment = recorder.record(state, request, Some(by.id)).await?;
    let mut left = amount;
    for (invoice, balance) in open {
        if left == 0 {
            break;
        }
        let part = left.min(balance);
        recorder
            .allocate(
                state,
                payment.id,
                AllocationTarget::Invoice(InvoiceId(invoice)),
                part,
            )
            .await?;
        left -= part;
    }
    Ok(payment.number.unwrap_or_default())
}
