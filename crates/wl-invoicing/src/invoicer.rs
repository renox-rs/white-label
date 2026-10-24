//! The module's [`Invoicer`] (wl-invoicing §6): how other modules draft and
//! read invoices through the kernel. [`crate::WlInvoicing`] provides it
//! with [`wl_kernel::invoicing::provide_invoicer`].

use renox::prelude::*;
use renox::state_machine::{Outcome, TransitionError};
use wl_kernel::after_commit::AfterCommit;
use wl_kernel::invoicing::{
    CreditLine, CreditNoteId, CreditNoteSummary, DisputeRequest, DisputeState, DraftLine,
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus, InvoiceSummary, Invoicer, ReopenReason,
    Reopened, Unallocate, WriteOffAnswer,
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

    fn update_draft<'a>(
        &'a self,
        db: &'a Db,
        id: InvoiceId,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, bool> {
        Box::pin(async move { replace_draft(db, id, draft).await })
    }

    fn discard_draft<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, bool> {
        Box::pin(async move { discard_draft(db, id).await })
    }

    fn discard_draft_in<'a>(
        &'a self,
        tx: &'a mut renox::db::Transaction,
        id: InvoiceId,
    ) -> InvoiceFuture<'a, bool> {
        Box::pin(async move { discard_draft_in(tx, id).await })
    }

    fn find<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move { Ok(Invoice::find(db, id.0).await?.map(|i| i.summary())) })
    }

    fn net_total<'a>(&'a self, db: &'a Db, id: InvoiceId) -> InvoiceFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(Invoice::find(db, id.0).await?.map(|i| i.net_total)) })
    }

    fn billing<'a>(
        &'a self,
        db: &'a Db,
        origin: &'a wl_kernel::Subject,
    ) -> InvoiceFuture<'a, Vec<InvoiceId>> {
        Box::pin(async move { billing(db, origin).await })
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

    fn unallocate<'a>(
        &'a self,
        state: &'a AppState,
        request: Unallocate,
    ) -> InvoiceFuture<'a, Reopened> {
        Box::pin(async move { crate::unallocate::run(state, request).await })
    }

    fn unallocate_all<'a>(
        &'a self,
        state: &'a AppState,
        id: InvoiceId,
        reason: ReopenReason,
    ) -> InvoiceFuture<'a, Vec<(wl_kernel::Subject, i64)>> {
        Box::pin(async move { crate::unallocate::unallocate_all(state, id, reason).await })
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

    fn overdue_of<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
    ) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async move {
            let rows = Invoice::query()
                .where_(Invoice::STATUS.eq(InvoiceStatus::Overdue))
                .where_(Invoice::PARTY_ID.eq(party.0))
                .where_(Invoice::BALANCE.gt(0))
                .order_by(Invoice::ID)
                .get(db)
                .await?;
            Ok(rows.iter().map(Invoice::summary).collect())
        })
    }

    fn open_of<'a>(&'a self, db: &'a Db, party: PartyId) -> InvoiceFuture<'a, Vec<InvoiceSummary>> {
        Box::pin(async move {
            let open = [
                InvoiceStatus::Issued,
                InvoiceStatus::PartiallyPaid,
                InvoiceStatus::Overdue,
            ];
            let rows = Invoice::query()
                .where_(Invoice::STATUS.is_in(open))
                .where_(Invoice::PARTY_ID.eq(party.0))
                .where_(Invoice::BALANCE.gt(0))
                .order_by(Invoice::ID)
                .get(db)
                .await?;
            let mut found: Vec<InvoiceSummary> = rows.iter().map(Invoice::summary).collect();
            found.sort_by_key(|invoice| (invoice.due_date, invoice.id.0));
            Ok(found)
        })
    }

    fn lines_of<'a>(
        &'a self,
        db: &'a Db,
        invoice: InvoiceId,
    ) -> InvoiceFuture<'a, Vec<wl_kernel::invoicing::InvoiceLineInfo>> {
        Box::pin(async move {
            Ok(lines_of(db, invoice.0)
                .await?
                .iter()
                .filter_map(|line| {
                    Some(wl_kernel::invoicing::InvoiceLineInfo {
                        position: i32::try_from(line.position).ok()?,
                        description: line.description.clone(),
                        gross: line.gross_amount,
                        origin: line.origin(),
                        vat_category: line.vat_category,
                    })
                })
                .collect())
        })
    }

    fn dispute<'a>(
        &'a self,
        state: &'a AppState,
        request: DisputeRequest,
    ) -> InvoiceFuture<'a, i64> {
        Box::pin(async move { crate::dispute::raise(state, request).await })
    }

    fn dispute_state<'a>(
        &'a self,
        db: &'a Db,
        dispute: i64,
    ) -> InvoiceFuture<'a, Option<DisputeState>> {
        Box::pin(async move {
            Ok(crate::dispute::Dispute::find(db, dispute)
                .await?
                .map(|d| DisputeState {
                    invoice: InvoiceId(d.invoice_id),
                    outcome: d.outcome,
                }))
        })
    }

    fn write_off<'a>(
        &'a self,
        state: &'a AppState,
        by: &'a AuthUser,
        id: InvoiceId,
        reason_code: &'a str,
        reason_text: Option<&'a str>,
    ) -> InvoiceFuture<'a, WriteOffAnswer> {
        Box::pin(async move {
            let invoice = Invoice::find_or_404(&state.db, id.0).await?;
            let outcome =
                crate::write_off::write_off(state, by, &invoice, reason_code, reason_text)
                    .await
                    .map_err(TransitionError::into_error)?;
            Ok(match outcome {
                Outcome::Done(_) => WriteOffAnswer::WrittenOff,
                Outcome::Pending(pending) => WriteOffAnswer::Pending {
                    request: pending
                        .data
                        .get(wl_kernel::approvals::submit::REQUEST_KEY)
                        .and_then(renox::serde_json::Value::as_i64)
                        .unwrap_or_default(),
                },
            })
        })
    }

    fn issue<'a>(
        &'a self,
        state: &'a AppState,
        id: InvoiceId,
        by: Option<&'a AuthUser>,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async move {
            let invoice = Invoice::find_or_404(&state.db, id.0).await?;
            let result = match by {
                Some(user) => crate::issue::issue(state, user, &invoice).await,
                None => crate::issue::issue_by_system(state, &invoice).await,
            };
            result.map_err(TransitionError::into_error)
        })
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
        // The plain method is the variant in a transaction of its own.
        Box::pin(async move {
            let mut tx = state.db.begin().await?;
            let mut after = AfterCommit::new();
            let note = self
                .credit_partly_in(state, &mut tx, &mut after, invoice, lines, reason, by)
                .await?;
            tx.commit().await?;
            after.run(state).await?;
            Ok(note)
        })
    }

    fn credit_in_full_in<'a>(
        &'a self,
        _state: &'a AppState,
        tx: &'a mut renox::db::Transaction,
        after: &'a mut AfterCommit,
        invoice: InvoiceId,
        reason: String,
        by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            let invoice = Invoice::find_or_404(&mut *tx, invoice.0).await?;
            let mut outcomes = crate::credit_notes::Outcomes::default();
            // The origins are released only by the back office's sheet.
            let note = crate::credit::credit_in_full_in(
                tx,
                &invoice,
                &reason,
                false,
                &By::System(by),
                &mut outcomes,
            )
            .await
            .map_err(TransitionError::into_error)?;
            outcomes.queue_on(after);
            Ok(note.credit_note_id())
        })
    }

    fn credit_partly_in<'a>(
        &'a self,
        _state: &'a AppState,
        tx: &'a mut renox::db::Transaction,
        after: &'a mut AfterCommit,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            let mut outcomes = crate::credit_notes::Outcomes::default();
            let note = crate::credit_notes::credit_partly_in(
                tx,
                invoice.0,
                &lines,
                &reason,
                &By::System(by),
                &mut outcomes,
            )
            .await
            .map_err(TransitionError::into_error)?;
            outcomes.queue_on(after);
            Ok(note.credit_note_id())
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
pub(crate) fn is_currency(currency: &str) -> bool {
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
    // Findable as a draft from the commit on (BL-AGT-04).
    wl_kernel::search::index_in(&mut tx, &invoice.subject()).await?;
    tx.commit().await?;
    Ok(invoice)
}

/// Deletes the `draft` invoice `id` and its lines ([`Invoicer::discard_draft`],
/// spec `discard`): a draft has no number and was never sent. Returns `false`
/// and changes nothing for an invoice that is not a draft or does not exist.
pub async fn discard_draft(db: &Db, id: InvoiceId) -> Result<bool> {
    let mut tx = db.begin().await?;
    let gone = discard_draft_in(&mut tx, id).await?;
    tx.commit().await?;
    Ok(gone)
}

/// [`discard_draft`] inside the caller's transaction `tx`
/// ([`Invoicer::discard_draft_in`], BL-KRN-11): nothing is committed, so a
/// rollback keeps the draft.
pub async fn discard_draft_in(tx: &mut renox::db::Transaction, id: InvoiceId) -> Result<bool> {
    let Some(invoice) = Invoice::find(&mut *tx, id.0).await? else {
        return Ok(false);
    };
    if invoice.status != InvoiceStatus::Draft {
        return Ok(false);
    }
    renox::db::sql("DELETE FROM wl_invoicing_invoice_lines WHERE invoice_id = ?")
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
    // Not the status column of the machine: a draft is gone, not moved.
    let gone =
        renox::db::sql("DELETE FROM wl_invoicing_invoices WHERE id = ? AND status = 'draft'")
            .bind(id.0)
            .execute(&mut *tx)
            .await?;
    // A discarded draft leaves the search index with it (BL-AGT-04).
    if gone > 0 {
        wl_kernel::search::remove_in(&mut *tx, &id.subject()).await?;
    }
    Ok(gone > 0)
}

/// Replaces the lines of the `draft` invoice `id` with those of `draft` and
/// recomputes its totals in one transaction ([`Invoicer::update_draft`]). The
/// party, source, entity and dates stay. Returns `false` and changes nothing
/// when the invoice is no longer a draft. Refused (422) for a line as
/// [`create_draft`] refuses; `NotFound` for an unknown invoice.
pub async fn replace_draft(db: &Db, id: InvoiceId, draft: InvoiceDraft) -> Result<bool> {
    if let Some(problem) = draft
        .lines
        .iter()
        .enumerate()
        .find_map(|(i, line)| line_problem(i + 1, line))
    {
        return Err(refused(problem));
    }
    let invoice = Invoice::find_or_404(db, id.0).await?;
    if invoice.status != InvoiceStatus::Draft {
        return Ok(false);
    }
    let rate = invoice.aed_rate();
    let mut lines: Vec<InvoiceLine> = draft
        .lines
        .iter()
        .zip(1..)
        .map(|(line, position)| InvoiceLine::from_draft(invoice.id, position, line, rate))
        .collect();
    let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
    let mut totals = invoice.clone();
    totals.set_totals(&VatTotals::of(&amounts));

    let mut tx = db.begin().await?;
    InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .delete(&mut tx)
        .await?;
    for line in &mut lines {
        line.insert(&mut tx).await?;
    }
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .where_(Invoice::STATUS.eq(InvoiceStatus::Draft))
        .update(
            &mut tx,
            &[
                ("net_total", &totals.net_total),
                ("vat_total", &totals.vat_total),
                ("gross_total", &totals.gross_total),
                ("vat_total_aed", &totals.vat_total_aed),
                ("balance", &totals.balance),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// The lines of invoice `invoice_id`, in order.
pub async fn lines_of<'c>(
    db: impl renox::db::Executor<'c>,
    invoice_id: i64,
) -> Result<Vec<InvoiceLine>> {
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

/// The invoices other than drafts with a line that bills `origin`, oldest
/// first ([`Invoicer::billing`]).
pub async fn billing(db: &Db, origin: &wl_kernel::Subject) -> Result<Vec<InvoiceId>> {
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::ORIGIN_TYPE.eq(origin.subject_type.clone()))
        .where_(InvoiceLine::ORIGIN_ID.eq(origin.subject_id))
        .order_by(InvoiceLine::INVOICE_ID)
        .get(db)
        .await?;
    let mut ids: Vec<i64> = lines.iter().map(|l| l.invoice_id).collect();
    ids.dedup();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let invoices = Invoice::query()
        .where_(Invoice::ID.is_in(ids))
        .where_(Invoice::STATUS.ne(InvoiceStatus::Draft))
        .order_by(Invoice::ID)
        .get(db)
        .await?;
    Ok(invoices.iter().map(Invoice::invoice_id).collect())
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
