//! Invoices and credit notes in the global search (BL-AGT-04,
//! `PLT.SRC.01`): found by their number, and by the customer's name, which
//! is the line under the number.
//!
//! **Scope.** An invoice is in the scope of the record it bills for (a
//! lease: its building) or of its scope unit (a building), so a property
//! manager of one building finds that building's invoices only; a credit
//! note is in its invoice's scope. The scope is read in the indexing
//! transaction: from the source's kernel machine when it has one with a
//! scope (a lease), else from the source's own search document.
//!
//! **Names.** The customer's name is the party's own search document (the
//! party module indexes it), or the name the invoice took at issue.
//!
//! The documents are written again after every transition (issue, payment,
//! write-off, credit: [`register`] listens for them) and in the transaction
//! that drafts or discards an invoice and drafts a credit note.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, kinds};

use crate::credit_note::CreditNote;
use crate::invoice::Invoice;
use crate::permissions;

/// Declares `invoicing.invoice` and `invoicing.credit_note` for search.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Invoice>(INVOICE_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Invoices")
        .link("/invoices/{id}")
        .portal_visible()
        .document_in(|tx, invoice| Box::pin(invoice_document(tx, invoice)))
        .register();
    Searchable::of::<CreditNote>(CREDIT_NOTE_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Credit notes")
        .link("/credit-notes/{id}")
        .document_in(|tx, note| Box::pin(credit_note_document(tx, note)))
        .register();
    app.listen(|event: Transitioned<Invoice>, state: AppState| async move {
        index(&state.db, &[Subject::new(INVOICE_SUBJECT_TYPE, event.key)]).await
    });
    app.listen(
        |event: Transitioned<CreditNote>, state: AppState| async move {
            index(&state.db, &[Subject::new(CREDIT_NOTE_SUBJECT_TYPE, event.key)]).await
        },
    );
}

/// The search document of `invoice`: its number (or "Draft invoice #id"
/// before issue) over the customer's name.
pub async fn invoice_document(tx: &mut Transaction, invoice: &Invoice) -> Result<SearchDocument> {
    let snapshot = invoice.party_snapshot.as_ref().map(|s| s.0.clone());
    let customer = match party_name_in(tx, invoice.party_id).await? {
        Some(name) => name,
        None => snapshot
            .as_ref()
            .map(|s| s.legal_name_en.clone())
            .unwrap_or_default(),
    };
    let number = invoice.number.clone().unwrap_or_default();
    let title = if number.is_empty() {
        format!("Draft invoice #{}", invoice.id)
    } else {
        number.clone()
    };
    let mut candidates = vec![invoice.source()];
    if let Some(unit) = invoice.scope() {
        candidates.push(unit);
    }
    let scope = scope_in(tx, &candidates).await?;
    Ok(SearchDocument::new(title)
        .secondary(customer)
        .keyword(
            snapshot
                .as_ref()
                .and_then(|s| s.legal_name_ar.clone())
                .unwrap_or_default(),
        )
        .identifier(kinds::NUMBER, number)
        .status(invoice.status.as_str())
        .scope(scope)
        .legal_entity(invoice.legal_entity_id))
}

/// The search document of credit note `note`: its number over the
/// customer's name, in its invoice's scope.
pub async fn credit_note_document(
    tx: &mut Transaction,
    note: &CreditNote,
) -> Result<SearchDocument> {
    let customer = match party_name_in(tx, note.party_id).await? {
        Some(name) => name,
        None => note
            .party_snapshot
            .as_ref()
            .map(|s| s.0.legal_name_en.clone())
            .unwrap_or_default(),
    };
    let number = note.number.clone().unwrap_or_default();
    let title = if number.is_empty() {
        format!("Draft credit note #{}", note.id)
    } else {
        number.clone()
    };
    let invoice = Subject::new(INVOICE_SUBJECT_TYPE, note.invoice_id);
    let invoice_number = stored_in(tx, &invoice).await?.map(|d| d.title);
    let scope = scope_in(tx, &[invoice]).await?;
    Ok(SearchDocument::new(title)
        .secondary(customer)
        .keyword(invoice_number.unwrap_or_default())
        .identifier(kinds::NUMBER, number)
        .status(note.status.as_str())
        .scope(scope)
        .legal_entity(note.legal_entity_id))
}

/// The stored search document of `subject`, read in `tx`.
async fn stored_in(tx: &mut Transaction, subject: &Subject) -> Result<Option<SearchDocumentRow>> {
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await
}

/// The name party `id` is found by: the title of its search document.
async fn party_name_in(tx: &mut Transaction, id: i64) -> Result<Option<String>> {
    Ok(stored_in(tx, &Subject::new(PARTY_SUBJECT_TYPE, id))
        .await?
        .map(|document| document.title)
        .filter(|name| !name.is_empty()))
}

/// The scope of the first of `candidates` that has one: its kernel
/// machine's scope (read in `tx`), else the scope of its search document;
/// global when none has one. (`wl_kernel::scopes::scope_of` reads outside
/// the transaction, which a single SQLite connection cannot do while the
/// indexing transaction is open.)
async fn scope_in(tx: &mut Transaction, candidates: &[Subject]) -> Result<Scope> {
    for subject in candidates {
        if let Some(machine) =
            wl_kernel::machines::get(&subject.subject_type).filter(|m| m.has_scope())
        {
            // A record that is gone (or a source a test made up) has no
            // scope: the machine answers `NotFound`.
            match machine.scope_in(tx, subject.subject_id).await {
                Ok(scope) if !scope.is_global() => return Ok(scope),
                Ok(_) | Err(Error::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(document) = stored_in(tx, subject).await?
            && !document.scope_type.is_empty()
        {
            return Ok(Scope::new(&document.scope_type, &document.scope_id));
        }
    }
    Ok(Scope::global())
}

/// Writes the documents of `subjects` again in their own transaction.
pub async fn index(db: &Db, subjects: &[Subject]) -> Result {
    let mut tx = db.begin().await?;
    for subject in subjects {
        wl_kernel::search::index_in(&mut tx, subject).await?;
    }
    tx.commit().await?;
    Ok(())
}
