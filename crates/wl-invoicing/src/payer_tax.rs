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
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory};

use crate::invoicer::create_draft;
use crate::payer_bill::{PayerBill, PayerBillLine, lines_of};
use crate::texts::english;

/// Whether `bill`, as it stands with `lines`, still needs a tax document.
pub(crate) fn needs_documents(bill: &PayerBill, _lines: &[PayerBillLine]) -> bool {
    bill.payer_is_recipient() && bill.payer_invoice_id.is_none()
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
    if bill.payer_is_recipient() && bill.payer_invoice_id.is_none() {
        issue_first(state, &bill, &lines).await?;
    } else {
        clear_flag(state, bill.id).await?;
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
            &mut *tx,
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
                &mut *tx,
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
