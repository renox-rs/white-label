//! The parties of a search document ([`SearchDocument::party`]), stored
//! beside it so the portal search (BL-AGT-05, [`super::portal`]) filters by
//! the portal user's parties in the statement.
//!
//! [`SearchDocument::party`]: super::SearchDocument::party

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::Serialize;

use crate::subject::Subject;

/// One party of one record, as stored.
#[derive(Debug, Clone, Default, Serialize, Model)]
#[model(
    table = "wl_search_parties",
    unique(subject_type, subject_id, party_id),
    index(party_id)
)]
pub struct SearchPartyRow {
    /// The primary key.
    pub id: i64,
    /// The record's type.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// The party the record belongs to.
    pub party_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Replaces the stored parties of `subject` with `parties`.
pub(super) async fn write_in(tx: &mut Transaction, subject: &Subject, parties: &[i64]) -> Result {
    remove_in(tx, subject).await?;
    for party_id in parties {
        let mut row = SearchPartyRow {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            party_id: *party_id,
            ..Default::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(())
}

/// Removes the stored parties of `subject`.
pub(super) async fn remove_in(tx: &mut Transaction, subject: &Subject) -> Result {
    SearchPartyRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .delete(&mut *tx)
        .await?;
    Ok(())
}

/// Removes the stored parties of every record of `subject_type`.
pub(super) async fn remove_type_in(tx: &mut Transaction, subject_type: &str) -> Result {
    SearchPartyRow::query()
        .where_eq("subject_type", subject_type)
        .delete(&mut *tx)
        .await?;
    Ok(())
}

/// The stored parties of `subject`.
pub async fn parties_of(db: &Db, subject: &Subject) -> Result<Vec<i64>> {
    Ok(SearchPartyRow::query()
        .where_eq("subject_type", subject.subject_type.as_str())
        .where_eq("subject_id", subject.subject_id)
        .order_by("party_id")
        .get(db)
        .await?
        .into_iter()
        .map(|row| row.party_id)
        .collect())
}
