//! Making a payer bill from a payer split, and correcting what a draft
//! claims (wl-invoicing §5.8 steps 1 and 6).
//!
//! The vertical computes the split (05 from the payer contract, 08 from the
//! insurer's approved amount) and asks for the bill through the kernel's
//! `PayerBills::create`. The bill starts as a `draft` under the payer
//! agreement: the issuing entity, the payer and who gets the tax invoice are
//! the agreement's. A source can be billed to one payer agreement once at a
//! time: a second bill needs the first one voided (the split is then
//! released and made again, `void`).
//!
//! A draft that came back from a technical rejection is corrected line by
//! line with [`correct_line`]; the tax documents follow at the next `issue`.

use renox::prelude::*;
use wl_kernel::payer_bills::{NewPayerBill, PayerBillStatus};

use crate::payer_agreement::PayerAgreement;
use crate::payer_bill::{PayerBill, PayerBillLine, Totals, lines_of, write_totals_in};
use crate::texts::english;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Makes a `draft` payer bill from `request`, by user `by` (`None` for the
/// system).
///
/// # Errors
///
/// 404 for an unknown agreement; 422 for an inactive agreement, a bill
/// without lines, a line that claims 0 or less, a blank description, or when
/// a payer bill of the agreement for the same source is not voided.
pub async fn create(state: &AppState, request: NewPayerBill, by: Option<i64>) -> Result<PayerBill> {
    let agreement = PayerAgreement::find_or_404(&state.db, request.agreement).await?;
    if !agreement.active {
        return Err(refused(english(
            "wl_invoicing.payer_bill.errors.agreement_inactive",
        )));
    }
    if request.lines.is_empty()
        || request
            .lines
            .iter()
            .any(|line| line.claimed <= 0 || line.description.trim().is_empty())
    {
        return Err(refused(english("wl_invoicing.payer_bill.errors.lines")));
    }
    let existing = PayerBill::query()
        .where_(PayerBill::SOURCE_TYPE.eq(request.source.subject_type.clone()))
        .where_(PayerBill::SOURCE_ID.eq(request.source.subject_id))
        .where_(PayerBill::AGREEMENT_ID.eq(agreement.id))
        .where_op("status", "!=", PayerBillStatus::Voided.as_str())
        .count(&state.db)
        .await?;
    if existing > 0 {
        return Err(refused(english(
            "wl_invoicing.payer_bill.errors.already_billed",
        )));
    }
    let mut lines: Vec<PayerBillLine> = request
        .lines
        .iter()
        .zip(1_i64..)
        .map(|(line, position)| {
            let mut made = PayerBillLine::new(position, &line.description, line.claimed)
                .vat_category(line.vat_category);
            made.invoice_line_position = line.invoice_line;
            made
        })
        .collect();
    let totals = Totals::of(&lines);
    let mut bill = PayerBill {
        legal_entity_id: agreement.legal_entity_id,
        agreement_id: agreement.id,
        payer_party_id: agreement.payer_party_id,
        customer_party_id: request.customer.0,
        source_type: request.source.subject_type.clone(),
        source_id: request.source.subject_id,
        vat_recipient: agreement.vat_recipient,
        payer_reference: request.payer_reference.clone(),
        currency: request.currency.clone(),
        service_date: request.service_date,
        customer_invoice_id: request.customer_invoice.map(|invoice| invoice.0),
        claimed_total: totals.claimed,
        balance: totals.balance,
        void_before_first_submission: request.void_before_first_submission,
        created_by: by,
        status: PayerBillStatus::Draft,
        ..Default::default()
    };
    let mut tx = state.db.begin().await?;
    bill.insert(&mut tx).await?;
    for line in &mut lines {
        line.payer_bill_id = bill.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(bill)
}

/// Changes what the `draft` bill `bill_id` claims on its line `line_id` to
/// `claimed` fils and rewrites the totals.
///
/// # Errors
///
/// 404 for an unknown bill; 422 when the bill is not a `draft`, the line is
/// not one of its lines, or the amount is 0 or less.
pub async fn correct_line(
    state: &AppState,
    bill_id: i64,
    line_id: i64,
    claimed: i64,
) -> Result<PayerBill> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    if bill.status != PayerBillStatus::Draft {
        return Err(refused(english("wl_invoicing.payer_bill.errors.not_draft")));
    }
    let lines = lines_of(&state.db, bill.id).await?;
    if !lines.iter().any(|line| line.id == line_id) {
        return Err(refused(english("wl_invoicing.payer_bill.errors.no_line")));
    }
    if claimed <= 0 {
        return Err(refused(english("wl_invoicing.payer_bill.errors.lines")));
    }
    let mut tx = state.db.begin().await?;
    PayerBillLine::query()
        .where_(PayerBillLine::ID.eq(line_id))
        .update(
            &mut tx,
            &[("claimed", &claimed), ("updated_at", &renox::db::now())],
        )
        .await?;
    let lines = lines_of(&mut tx, bill.id).await?;
    write_totals_in(&mut tx, bill.id, &lines).await?;
    tx.commit().await?;
    PayerBill::find_or_404(&state.db, bill_id).await
}
