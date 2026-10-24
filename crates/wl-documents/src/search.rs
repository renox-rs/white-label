//! Documents in the global search (BL-AGT-04, `PLT.SRC.01`): found by
//! their title and kind, over the record they are attached to; a document
//! number (a passport, a trade licence) is an identifier stored only as a
//! hash, so it matches on the exact number and never appears in the text.
//!
//! A document is in the scope of its record (a lease or a unit: the
//! building), read in the indexing transaction from that record's kernel
//! machine or search document, and carries its kind's data class.
//!
//! A result opens the kernel's page of the document's related records (the
//! module has no page per document; the record's documents page needs the
//! record). The documents of any record are this module's generic relation
//! (`lib.rs`).

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, Sensitivity, kinds};

use crate::documents::Document;
use crate::expiry::renewal::DOCUMENT_SUBJECT_TYPE;
use crate::kinds::{DocumentKind, Sensitivity as KindSensitivity};
use crate::permissions;

/// Declares `wl_documents.document` for search and keeps its document
/// current after each transition.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Document>(DOCUMENT_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Documents")
        .document_in(|tx, document| Box::pin(document_in(tx, document)))
        .register();
    app.listen(|event: Transitioned<Document>, state: AppState| async move {
        index(&state.db, event.key).await
    });
}

/// The search document of `document`.
pub async fn document_in(tx: &mut Transaction, document: &Document) -> Result<SearchDocument> {
    let kind = DocumentKind::query()
        .where_eq("id", document.kind_id)
        .first(&mut *tx)
        .await?;
    let attached_to = document.subject();
    let record = stored_in(tx, &attached_to)
        .await?
        .map(|row| row.title)
        .unwrap_or_default();
    let scope = scope_in(tx, &[attached_to]).await?;
    let (kind_name, sensitivity) = match &kind {
        Some(kind) => (kind.name_en.clone(), class_of(kind.sensitivity)),
        None => (String::new(), Sensitivity::Normal),
    };
    let line = [kind_name.as_str(), record.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    // Always hashed: the number of an identity document is personal data,
    // and a kind's class is not always set (the seeds are `normal`).
    let number_class = if sensitivity.is_sensitive() {
        sensitivity
    } else {
        Sensitivity::Personal
    };
    Ok(SearchDocument::new(document.title.clone())
        .secondary(line)
        .identifier_of_class(
            kinds::NUMBER,
            document.number.clone().unwrap_or_default(),
            number_class,
        )
        .status(document.status.as_str())
        .scope(scope)
        .sensitivity(sensitivity))
}

/// The search index's data class of a kind's class.
fn class_of(sensitivity: KindSensitivity) -> Sensitivity {
    match sensitivity {
        KindSensitivity::Normal => Sensitivity::Normal,
        KindSensitivity::Personal => Sensitivity::Personal,
        KindSensitivity::Health => Sensitivity::Health,
        KindSensitivity::Financial => Sensitivity::Financial,
    }
}

/// The stored search document of `subject`, read in `tx`.
async fn stored_in(tx: &mut Transaction, subject: &Subject) -> Result<Option<SearchDocumentRow>> {
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(&mut *tx)
        .await
}

/// The scope of the first of `candidates` that has one: its kernel
/// machine's scope (read in `tx`), else the scope of its search document;
/// global when none has one.
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

/// Writes the search document of document `id` again in its own
/// transaction.
pub async fn index(db: &Db, id: i64) -> Result {
    let mut tx = db.begin().await?;
    wl_kernel::search::index_in(&mut tx, &Subject::new(DOCUMENT_SUBJECT_TYPE, id)).await?;
    tx.commit().await?;
    Ok(())
}
