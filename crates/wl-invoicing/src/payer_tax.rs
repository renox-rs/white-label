//! The tax documents of a payer bill (wl-invoicing §4 *Payer bill*,
//! `FIN.ACL.01`): who gets a tax invoice for what the payer is asked to pay.
//!
//! - **The payer is the VAT recipient.** `issue` issues a tax invoice to the
//!   payer for the claimed lines, once: a payer bill that was issued before
//!   keeps the invoice it has. The invoice is addressed to the payer, has
//!   the payer bill as its source and carries the same lines in the same
//!   order, so a line's position on the invoice is the payer bill line's.
//! - **The customer is the VAT recipient.** The customer's tax invoice was
//!   issued for the full supply when the customer's share was billed, so it
//!   already carries the payer lines; the payer bill only records the payer
//!   receivable share on it and no tax document is made here.
//!
//! - **A corrected amount** (a bill sent back by `submission_rejected` and
//!   issued again with other claimed amounts) never edits the payer's tax
//!   invoice and never issues it again in full (`FIN.ACL.01`): a lower
//!   amount is a credit note for the difference against the invoice, a
//!   higher one an extra tax invoice for the difference only. The invoice
//!   keeps what it billed (`PayerBillLine::invoiced`), so each difference
//!   is documented once.
//!
//! The documents are made after the transition has committed, by
//! [`settle`], because a transition's effect cannot call another module's
//! contract. The transition leaves `tax_document_pending` on the bill until
//! [`settle`] has made them, so a bill whose document failed shows it and
//! the next [`settle`] (the next issue, or a retry) makes it. The invoice is
//! issued with [`crate::issue::issue_for_payer_bill`], under the payer
//! bill's own permission.

use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::entities::is_trn;
use wl_kernel::invoicing::{CreditLine, DraftLine, InvoiceDraft, VatCategory};

use crate::credit_notes::{self, By};
use crate::invoicer::create_draft;
use crate::payer_bill::{PayerBill, PayerBillLine, lines_of};
use crate::texts::english;

/// Whether `bill`, as it stands with `lines`, still needs a tax document:
/// the payer's invoice is not issued yet, or a line claims another amount
/// than the invoice billed for it.
pub(crate) fn needs_documents(bill: &PayerBill, lines: &[PayerBillLine]) -> bool {
    bill.payer_is_recipient()
        && (bill.payer_invoice_id.is_none() || lines.iter().any(|l| l.invoiced != l.claimed))
}

/// Why the payer's tax invoice for `bill` could not be issued now, if it
/// could not: the issuing entity is not active, or not VAT registered while
/// a line is standard-rated, or the payer is not found. These are the
/// guards of the invoice's own `issue`, asked before the payer bill moves.
/// `None` when the customer is the recipient (no invoice is made).
pub(crate) async fn issue_problem(
    state: &AppState,
    bill: &PayerBill,
    lines: &[PayerBillLine],
) -> Result<Option<String>> {
    if !bill.payer_is_recipient() || bill.payer_invoice_id.is_some() {
        return Ok(None);
    }
    let entity = LegalEntity::find(&state.db, bill.legal_entity_id).await?;
    let Some(entity) = entity.filter(LegalEntity::is_active) else {
        return Ok(Some(
            english("wl_invoicing.payer_bill.errors.entity_inactive").to_owned(),
        ));
    };
    let standard = lines
        .iter()
        .any(|line| line.vat_category == VatCategory::Standard);
    let registered =
        entity.vat_registered && entity.trn.as_deref().is_some_and(|trn| is_trn(trn.trim()));
    if standard && !registered {
        return Ok(Some(
            english("wl_invoicing.payer_bill.errors.entity_not_registered").to_owned(),
        ));
    }
    let payer = wl_kernel::parties::directory(state)
        .billing_identity(&state.db, bill.payer())
        .await?;
    if payer.is_none() {
        return Ok(Some(
            english("wl_invoicing.payer_bill.errors.payer_not_found").to_owned(),
        ));
    }
    Ok(None)
}

/// Makes the tax documents `bill_id` still needs and clears its
/// `tax_document_pending` flag. Nothing is made for a bill that needs none.
///
/// # Errors
///
/// The refusal of the invoice's own `issue` or of the credit note; the flag
/// stays set then.
pub(crate) async fn settle(state: &AppState, bill_id: i64) -> Result {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    if !bill.tax_document_pending {
        return Ok(());
    }
    let lines = lines_of(&state.db, bill.id).await?;
    match bill.payer_invoice_id {
        None if bill.payer_is_recipient() => issue_first(state, &bill, &lines).await?,
        Some(invoice_id) if bill.payer_is_recipient() => {
            adjust(state, &bill, invoice_id, &lines).await?;
            clear_flag(state, bill.id).await?;
        }
        _ => clear_flag(state, bill.id).await?,
    }
    Ok(())
}

/// Writes `invoiced = claimed` on `lines`.
async fn mark_invoiced(state: &AppState, lines: &[&PayerBillLine]) -> Result {
    for line in lines {
        PayerBillLine::query()
            .where_(PayerBillLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("invoiced", &line.claimed),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// Documents a corrected amount against the payer's tax invoice
/// `invoice_id`: a credit note for the lines that now claim less, an extra
/// tax invoice for the lines that now claim more, each only for the
/// difference. A step is recorded on the lines as soon as it is made, so a
/// retry after a failure of the next step makes nothing twice.
async fn adjust(
    state: &AppState,
    bill: &PayerBill,
    invoice_id: i64,
    lines: &[PayerBillLine],
) -> Result {
    let lower: Vec<&PayerBillLine> = lines.iter().filter(|l| l.invoiced > l.claimed).collect();
    let credit: Vec<CreditLine> = lower
        .iter()
        .filter_map(|l| Some(CreditLine::new(l.invoice_line_position?, l.invoiced - l.claimed)))
        .collect();
    if !credit.is_empty() {
        let reason = format!(
            "Payer bill {}: corrected amount",
            bill.number.as_deref().unwrap_or("-")
        );
        credit_notes::credit_partly(state, invoice_id, &credit, &reason, By::System(None))
            .await
            .map_err(renox::state_machine::TransitionError::into_error)?;
        mark_invoiced(state, &lower).await?;
    }
    let higher: Vec<&PayerBillLine> = lines.iter().filter(|l| l.claimed > l.invoiced).collect();
    if !higher.is_empty() {
        let extra: Vec<(&PayerBillLine, i64)> =
            higher.iter().map(|l| (*l, l.claimed - l.invoiced)).collect();
        let invoice = create_draft(
            &state.db,
            draft_for(bill, &extra, " (additional amount)"),
        )
        .await?;
        crate::issue::issue_for_payer_bill(state, &invoice)
            .await
            .map_err(renox::state_machine::TransitionError::into_error)?;
        mark_invoiced(state, &higher).await?;
    }
    Ok(())
}

/// Clears `tax_document_pending` on bill `id`.
async fn clear_flag(state: &AppState, id: i64) -> Result {
    PayerBill::query()
        .where_(PayerBill::ID.eq(id))
        .update(
            &state.db,
            &[
                ("tax_document_pending", &false),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The draft of a tax invoice to the payer for `lines` (their claimed
/// amounts, or `amounts` when given) in the order of the lines.
fn draft_for(bill: &PayerBill, lines: &[(&PayerBillLine, i64)], suffix: &str) -> InvoiceDraft {
    let mut draft = InvoiceDraft::new(bill.payer(), bill.subject())
        .legal_entity(bill.legal_entity_id)
        .currency(&bill.currency)
        .date_of_supply(bill.service_date);
    for (line, amount) in lines {
        draft = draft.line(DraftLine::new(
            &format!("{}{suffix}", line.description),
            *amount,
            line.vat_category,
        ));
    }
    draft
}

/// Issues the payer's tax invoice for the claimed lines and links it.
async fn issue_first(state: &AppState, bill: &PayerBill, lines: &[PayerBillLine]) -> Result {
    let claimed: Vec<(&PayerBillLine, i64)> = lines.iter().map(|l| (l, l.claimed)).collect();
    let invoice = create_draft(&state.db, draft_for(bill, &claimed, "")).await?;
    crate::issue::issue_for_payer_bill(state, &invoice)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    let mut tx = state.db.begin().await?;
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut tx,
            &[
                ("payer_invoice_id", &Some(invoice.id)),
                ("tax_document_pending", &false),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    for (position, line) in (1_i64..).zip(lines) {
        PayerBillLine::query()
            .where_(PayerBillLine::ID.eq(line.id))
            .update(
                &mut tx,
                &[
                    ("invoiced", &line.claimed),
                    ("invoice_line_position", &Some(position)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
