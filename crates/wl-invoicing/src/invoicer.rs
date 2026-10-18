//! The module's [`Invoicer`] (wl-invoicing §6): how other modules draft and
//! read invoices through the kernel. [`crate::WlInvoicing`] provides it
//! with [`wl_kernel::invoicing::provide_invoicer`].

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteId, CreditNoteSummary, DisputeRequest, DraftLine, InvoiceDraft,
    InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer,
};
use wl_kernel::{LegalEntity, PartyId};

use crate::credit_note::CreditNote;
use crate::credit_notes::By;
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::vat::{self, VatTotals};

/// Drafts and reads `wl_invoicing_invoices` for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Invoices;

impl Invoicer for Invoices {
    fn draft<'a>(
        &'a self,
        db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move { create_draft(db, draft).await.map(|i| Some(i.invoice_id())) })
    }

    fn find<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move { Ok(Invoice::find(db, id.0).await?.map(|i| i.summary())) })
    }

    fn allocate<'a>(
        &'a self,
        state: &'a AppState,
        id: InvoiceId,
        source: wl_kernel::Subject,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            let invoice = Invoice::find_or_404(&state.db, id.0).await?;
            // A written-off invoice is recovered, an open one allocated.
            let result = if invoice.status == InvoiceStatus::WrittenOff {
                crate::recover::recover(state, &invoice, &source, amount, None).await
            } else {
                crate::allocate::allocate(state, &invoice, &source, amount, None).await
            };
            result.map_err(TransitionError::into_error)
        })
    }

    fn written_off_of<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async move {
            let rows = Invoice::query()
                .where_(Invoice::STATUS.eq(InvoiceStatus::WrittenOff))
                .where_(Invoice::PARTY_ID.eq(party.0))
                .where_(Invoice::CURRENCY.eq(currency))
                .where_(Invoice::BALANCE.gt(0))
                .order_by(Invoice::ID)
                .get(db)
                .await?;
            Ok(rows.iter().map(Invoice::summary).collect())
        })
    }

    fn dispute<'a>(
        &'a self,
        state: &'a AppState,
        request: DisputeRequest,
    ) -> InvoiceFuture<'a, i64> {
        Box::pin(async move { crate::dispute::raise(state, request).await })
    }

    fn credit_in_full<'a>(
        &'a self,
        state: &'a AppState,
        invoice: InvoiceId,
        reason: String,
        by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            let invoice = Invoice::find_or_404(&state.db, invoice.0).await?;
            // The origins are released only by the back office's sheet.
            crate::credit::credit_in_full(state, &invoice, &reason, false, By::System(by))
                .await
                .map(|note| note.credit_note_id())
                .map_err(TransitionError::into_error)
        })
    }

    fn credit_partly<'a>(
        &'a self,
        state: &'a AppState,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            crate::credit_notes::credit_partly(state, invoice.0, &lines, &reason, By::System(by))
                .await
                .map(|note| note.credit_note_id())
                .map_err(TransitionError::into_error)
        })
    }

    fn find_credit_note<'a>(
        &'a self,
        db: &'a Db,
        id: CreditNoteId,
    ) -> InvoiceFuture<'a, Option<CreditNoteSummary>> {
        Box::pin(async move { Ok(CreditNote::find(db, id.0).await?.map(|n| n.summary())) })
    }

    fn credit_notes_for<'a>(
        &'a self,
        db: &'a Db,
        invoice: InvoiceId,
    ) -> InvoiceFuture<'a, Vec<CreditNoteSummary>> {
        Box::pin(async move {
            let notes = crate::credit_notes::for_invoice(db, invoice.0).await?;
            Ok(notes.iter().map(CreditNote::summary).collect())
        })
    }

    fn open_credit<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        currency: &'a str,
    ) -> InvoiceFuture<'a, i64> {
        Box::pin(async move { crate::credit_notes::open_credit(db, party.0, currency).await })
    }

    fn take_credit_for_refund<'a>(
        &'a self,
        state: &'a AppState,
        credit_note: CreditNoteId,
        refund: wl_kernel::Subject,
        attempt: i64,
        amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            crate::refund::take_for_refund(state, credit_note.0, &refund, attempt, amount)
                .await
                .map_err(TransitionError::into_error)
        })
    }
}

/// A 422 with `message`.
fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// Why `line` (number `position`, from 1) cannot be drafted, if it cannot.
fn line_problem(position: usize, line: &DraftLine) -> Option<String> {
    if line.description.trim().is_empty() {
        return Some(format!("line {position} has no description"));
    }
    if line.quantity_milli <= 0 {
        return Some(format!("line {position} has a quantity of 0 or less"));
    }
    if line.unit_price < 0 || line.discount < 0 {
        return Some(format!("line {position} has a negative price or discount"));
    }
    if vat::line_net(line.quantity_milli, line.unit_price, line.discount) < 0 {
        return Some(format!("line {position} has a discount above its amount"));
    }
    None
}

/// Whether `currency` is an ISO code like `AED`.
fn is_currency(currency: &str) -> bool {
    currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())
}

/// Creates a `draft` invoice with its lines and totals in one transaction.
/// The issuing entity is the draft's, else the scope's, else the default
/// one. Refused (422) for an unknown currency or a line with no
/// description, a quantity ≤ 0, a negative price or discount, or a discount
/// above the line's amount.
pub async fn create_draft(db: &Db, draft: InvoiceDraft) -> Result<Invoice> {
    let currency = draft.currency.trim().to_ascii_uppercase();
    if !is_currency(&currency) {
        return Err(refused(format!(
            "`{}` is not a currency code like AED",
            draft.currency
        )));
    }
    if let Some(problem) = draft
        .lines
        .iter()
        .enumerate()
        .find_map(|(i, line)| line_problem(i + 1, line))
    {
        return Err(refused(problem));
    }
    let legal_entity_id = match draft.legal_entity_id {
        Some(id) => LegalEntity::find_or_404(db, id).await?.id,
        None => {
            let scope = draft
                .scope
                .as_ref()
                .map(|s| (s.subject_type.as_str(), s.subject_id));
            LegalEntity::for_scope(db, scope).await?.id
        }
    };
    let mut invoice = Invoice {
        legal_entity_id: Some(legal_entity_id),
        party_id: draft.party.0,
        source_type: draft.source.subject_type.clone(),
        source_id: draft.source.subject_id,
        scope_type: draft.scope.as_ref().map(|s| s.subject_type.clone()),
        scope_id: draft.scope.as_ref().map(|s| s.subject_id),
        currency,
        due_date: draft.due_date,
        date_of_supply: draft.date_of_supply,
        status: InvoiceStatus::Draft,
        ..Default::default()
    };
    let rate = invoice.aed_rate();
    let mut lines: Vec<InvoiceLine> = draft
        .lines
        .iter()
        .zip(1..)
        .map(|(line, position)| InvoiceLine::from_draft(0, position, line, rate))
        .collect();
    let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
    invoice.set_totals(&VatTotals::of(&amounts));

    let mut tx = db.begin().await?;
    invoice.insert(&mut tx).await?;
    for line in &mut lines {
        line.invoice_id = invoice.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(invoice)
}

/// The lines of invoice `invoice_id`, in order.
pub async fn lines_of(db: &Db, invoice_id: i64) -> Result<Vec<InvoiceLine>> {
    InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice_id))
        .order_by(InvoiceLine::POSITION)
        .get(db)
        .await
}

/// The totals per VAT category of invoice `invoice_id`, from its stored
/// lines.
pub async fn totals_of(db: &Db, invoice_id: i64) -> Result<VatTotals> {
    let lines = lines_of(db, invoice_id).await?;
    let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
    Ok(VatTotals::of(&amounts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    #[test]
    fn lines_are_checked_before_drafting() {
        let ok = DraftLine::new("Rent", 100_000, VatCategory::Exempt);
        assert_eq!(line_problem(1, &ok), None);
        let blank = DraftLine::new("  ", 100, VatCategory::Standard);
        assert!(line_problem(1, &blank).unwrap().contains("description"));
        let none = ok.clone().quantity_milli(0);
        assert!(line_problem(2, &none).unwrap().contains("line 2"));
        let negative = DraftLine::new("Fee", -1, VatCategory::Standard);
        assert!(line_problem(1, &negative).unwrap().contains("negative"));
        let too_much = DraftLine::new("Fee", 100, VatCategory::Standard).discount(101);
        assert!(line_problem(1, &too_much).unwrap().contains("discount"));
    }

    #[test]
    fn currencies_are_iso_codes() {
        assert!(is_currency("AED"));
        assert!(is_currency("USD"));
        assert!(!is_currency("aed"));
        assert!(!is_currency("DIRHAM"));
        assert!(!is_currency(""));
    }
}
