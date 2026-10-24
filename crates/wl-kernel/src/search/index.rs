//! Writing the index: one record ([`index_in`], [`remove_in`]), a whole type
//! or every type ([`rebuild_type`], [`rebuild`]), an undone import
//! ([`undo_import`]); and reading it back ([`document_of`],
//! [`lookup_identifier`]).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use renox::db::{DbValue, Transaction, sql};
use renox::import::{ImportChange, Undo, UndoReport};
use renox::prelude::*;

use super::document::{hash, normalize_identifier};
use super::order::rebuild_order;
use super::registry::{self, Searchable};
use super::{SearchDocumentRow, SearchIdentifierRow, kinds};
use crate::subject::Subject;

/// How many records [`rebuild_type`] reads at a time.
const REBUILD_PAGE: u64 = 500;

/// Writes the search document of `subject` in `tx`, the transaction that
/// changed the record, so a rolled back change leaves no document. A
/// record that no longer exists loses its document. Returns `false`, and
/// writes nothing, when the subject's type is not searchable.
pub async fn index_in(tx: &mut Transaction, subject: &Subject) -> Result<bool> {
    let Some(searchable) = registry::get(&subject.subject_type) else {
        return Ok(false);
    };
    match searchable.document_in(tx, subject.subject_id).await? {
        Some(document) => {
            let parties = document.parties().to_vec();
            let (row, identifiers) = document.into_rows(
                subject,
                searchable.permission(),
                searchable.is_portal_visible(),
            );
            write_in(tx, row, identifiers).await?;
            super::parties::write_in(tx, subject, &parties).await?;
        }
        None => remove_in(tx, subject).await?,
    }
    Ok(true)
}

/// Removes the document and identifiers of `subject` (a deleted record).
pub async fn remove_in(tx: &mut Transaction, subject: &Subject) -> Result {
    super::parties::remove_in(tx, subject).await?;
    SearchIdentifierRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .delete(&mut *tx)
        .await?;
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .delete(&mut *tx)
        .await?;
    Ok(())
}

/// Replaces the stored rows of one subject.
async fn write_in(
    tx: &mut Transaction,
    mut row: SearchDocumentRow,
    identifiers: Vec<SearchIdentifierRow>,
) -> Result {
    let existing = SearchDocumentRow::query()
        .where_eq("subject_type", row.subject_type.as_str())
        .where_eq("subject_id", row.subject_id)
        .first(&mut *tx)
        .await?;
    match existing {
        Some(existing) => {
            row.id = existing.id;
            row.created_at = existing.created_at;
            row.save(&mut *tx).await?;
        }
        None => row.insert(&mut *tx).await?,
    }
    SearchIdentifierRow::query()
        .where_eq("subject_type", row.subject_type.as_str())
        .where_eq("subject_id", row.subject_id)
        .delete(&mut *tx)
        .await?;
    for mut identifier in identifiers {
        identifier.insert(&mut *tx).await?;
    }
    Ok(())
}

/// The stored document of `subject`, if it has one.
pub async fn document_of(db: &Db, subject: &Subject) -> Result<Option<SearchDocumentRow>> {
    SearchDocumentRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .first(db)
        .await
}

/// The stored identifiers of `subject`, by kind.
pub async fn identifiers_of(db: &Db, subject: &Subject) -> Result<Vec<SearchIdentifierRow>> {
    SearchIdentifierRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .order_by("kind")
        .order_by("id")
        .get(db)
        .await
}

/// A record an identifier matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierMatch {
    /// The record.
    pub subject: Subject,
    /// Which kind of identifier matched.
    pub kind: String,
}

/// The records whose identifiers equal `text` as typed, in any form
/// ([`normalize_identifier`]), sensitive ones included (they match only on
/// the exact value). Every record that matches is returned, **unfiltered**:
/// the caller filters by permission, scope and record rules before it shows
/// or counts anything (BL-AGT-02).
pub async fn lookup_identifier(db: &Db, text: &str) -> Result<Vec<IdentifierMatch>> {
    let plain: BTreeSet<String> = [
        normalize_identifier("", text),
        normalize_identifier(kinds::PHONE, text),
        normalize_identifier(kinds::EMAIL, text),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect();
    if plain.is_empty() {
        return Ok(Vec::new());
    }
    // The hashes depend on the kind; ask only the kinds that have one.
    let hashed_kinds = sql("SELECT DISTINCT kind FROM wl_search_identifiers WHERE hashed = ?")
        .bind(true)
        .fetch_all(db)
        .await?
        .iter()
        .map(|row| row.try_get::<String>("kind"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut hashes = BTreeSet::new();
    for kind in &hashed_kinds {
        let normalized = normalize_identifier(kind, text);
        if !normalized.is_empty() {
            hashes.insert(hash(kind, &normalized));
        }
    }
    let mut rows = SearchIdentifierRow::query()
        .where_eq("hashed", false)
        .where_in("value", plain.iter().cloned().collect::<Vec<_>>())
        .get(db)
        .await?;
    if !hashes.is_empty() {
        rows.extend(
            SearchIdentifierRow::query()
                .where_eq("hashed", true)
                .where_in("value", hashes.into_iter().collect::<Vec<_>>())
                .get(db)
                .await?,
        );
    }
    let mut seen = BTreeSet::new();
    Ok(rows
        .into_iter()
        .filter(|row| seen.insert((row.subject_type.clone(), row.subject_id)))
        .map(|row| IdentifierMatch {
            subject: Subject::new(&row.subject_type, row.subject_id),
            kind: row.kind,
        })
        .collect())
}

/// Builds the documents of `subject_type` again from its records, in one
/// transaction: documents of records that no longer exist go. Returns the
/// number of documents written.
///
/// # Errors
///
/// `BadRequest` when the type is not searchable.
pub async fn rebuild_type(db: &Db, subject_type: &str) -> Result<u64> {
    let searchable = registry::get(subject_type)
        .ok_or_else(|| Error::BadRequest(format!("`{subject_type}` is not searchable")))?;
    let mut tx = db.begin().await?;
    let count = rebuild_in(&mut tx, &searchable).await?;
    tx.commit().await?;
    Ok(count)
}

/// Builds every registered type again ([`rebuild_type`]), with the number
/// of documents of each, in [`rebuild_order`]: a type after the types it
/// declared with [`SearchableBuilder::after`](super::SearchableBuilder::after)
/// (the parties before the invoices that read their names), else sorted by
/// subject type.
pub async fn rebuild(db: &Db) -> Result<Vec<(String, u64)>> {
    let mut counts = Vec::new();
    for subject_type in rebuild_sequence() {
        let count = rebuild_type(db, &subject_type).await?;
        counts.push((subject_type, count));
    }
    Ok(counts)
}

/// The registered types in the order [`rebuild`] builds them.
pub fn rebuild_sequence() -> Vec<String> {
    let types: Vec<(String, Vec<String>)> = registry::registered_types()
        .iter()
        .map(|searchable| {
            (
                searchable.subject_type().to_owned(),
                searchable.after().to_vec(),
            )
        })
        .collect();
    rebuild_order(&types)
}

async fn rebuild_in(tx: &mut Transaction, searchable: &Searchable) -> Result<u64> {
    let subject_type = searchable.subject_type();
    super::parties::remove_type_in(tx, subject_type).await?;
    SearchIdentifierRow::query()
        .where_eq("subject_type", subject_type)
        .delete(&mut *tx)
        .await?;
    SearchDocumentRow::query()
        .where_eq("subject_type", subject_type)
        .delete(&mut *tx)
        .await?;
    let mut count = 0;
    let mut after = 0;
    loop {
        let ids = searchable.ids_in(tx, after, REBUILD_PAGE).await?;
        let Some(&last) = ids.last() else { break };
        for id in ids {
            if index_in(tx, &Subject::new(subject_type, id)).await? {
                count += 1;
            }
        }
        after = last;
    }
    Ok(count)
}

/// Undoes import batch `batch_id` like `renox::import::undo`, and removes,
/// in the same transaction, the documents of the rows it deletes; the rows
/// it puts back as they were are indexed again once the undo is committed.
/// The import itself indexes the rows it writes with [`index_in`] on its
/// transaction.
pub async fn undo_import(state: &AppState, batch_id: i64) -> Result<UndoReport> {
    let restored: Arc<Mutex<Vec<Subject>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&restored);
    let report = Undo::batch(batch_id)
        .can_undo(move |tx, change| {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                let subjects = forget_import_change(tx, change).await?;
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend(subjects);
                Ok(())
            })
        })
        .run(state)
        .await?;
    let restored = std::mem::take(&mut *restored.lock().unwrap_or_else(PoisonError::into_inner));
    if !restored.is_empty() {
        let mut tx = state.db.begin().await?;
        for subject in &restored {
            index_in(&mut tx, subject).await?;
        }
        tx.commit().await?;
    }
    Ok(report)
}

/// For an app that undoes imports with its own `can_undo` check: call this
/// from it. Removes the documents of a row the undo deletes (an inserted row
/// of a searchable table) and returns the subjects of a row it puts back,
/// to index again after the commit.
pub async fn forget_import_change(
    tx: &mut Transaction,
    change: &ImportChange,
) -> Result<Vec<Subject>> {
    let DbValue::Integer(id) = change.key() else {
        return Ok(Vec::new());
    };
    let mut restored = Vec::new();
    for searchable in registry::by_table(change.table()) {
        let subject = Subject::new(searchable.subject_type(), *id);
        if change.is_insert() {
            remove_in(tx, &subject).await?;
        } else {
            restored.push(subject);
        }
    }
    Ok(restored)
}
