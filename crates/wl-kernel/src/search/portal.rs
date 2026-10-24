//! The portal search (BL-AGT-05, wl-kernel §5.8 point 2): what a tenant or
//! a client finds of **their own** records, by number or words, in the same
//! index as the staff search.
//!
//! Two filters, both in the statement, before anything is limited:
//!
//! - the type: only types registered
//!   [`portal_visible`](super::SearchableBuilder::portal_visible) (internal
//!   notes and internal documents never are), among those the caller allows
//!   for each party (the grant's portal permissions, the client's setting);
//! - the party: only documents whose record belongs to one of the portal
//!   user's parties ([`SearchDocument::party`](super::SearchDocument::party)).
//!   The staff permissions and scopes play no part: a portal user holds none.
//!
//! A user acting for several parties (a guardian, a company contact) gets
//! the records of each, each hit with the party it was found for.

use std::collections::BTreeMap;

use renox::db::{DbValue, Dialect, sql};
use renox::prelude::*;
use serde::Serialize;

use super::document::normalize_identifier;
use super::index::lookup_identifier;
use super::query::{FTS_TABLE, Statement, fts_query, mask, searchable_query};
use super::registry::{self, Searchable};
use super::{SearchDocumentRow, kinds};
use crate::subject::Subject;

/// The most results of a portal search.
pub const PORTAL_LIMIT: u64 = 50;

/// What one portal user may find for one party: the party and the
/// portal-visible types allowed for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalReach {
    /// The party the user acts for.
    pub party_id: i64,
    /// The subject types the user may find for it.
    pub types: Vec<String>,
}

/// One record found in the portal.
#[derive(Debug, Clone, Serialize)]
pub struct PortalHit {
    /// The record.
    pub subject: Subject,
    /// The party it was found for.
    pub party_id: i64,
    /// What it is called, e.g. `INV-PGP-2026-0004`.
    pub title: String,
    /// The line under the title.
    pub secondary: String,
    /// Its status, if the module gives one.
    pub status: Option<String>,
    /// Whether it was voided.
    pub voided: bool,
    /// The group it is shown in (`Invoices`).
    pub label: String,
    /// Where it opens in the portal.
    pub url: String,
    /// Whether an identifier of the record equals the query.
    pub identifier_match: bool,
    /// A sensitive identifier that matched, masked.
    pub matched: Option<String>,
}

/// The registered portal-visible types, sorted by subject type.
pub fn portal_types() -> Vec<Searchable> {
    registry::registered_types()
        .into_iter()
        .filter(Searchable::is_portal_visible)
        .collect()
}

/// The records of the parties of `reach` that match `text`, best first (an
/// identifier match, then the full-text rank, then the most recent), at
/// most `limit`. Empty when the query is too short or `reach` allows no
/// type for any party.
pub async fn portal_search(
    db: &Db,
    reach: &[PortalReach],
    text: &str,
    limit: u64,
) -> Result<Vec<PortalHit>> {
    let query = text.trim();
    let visible: Vec<String> = portal_types()
        .iter()
        .map(|t| t.subject_type().to_owned())
        .collect();
    // Only portal-visible types count, whatever the caller passed.
    let reach: Vec<(i64, Vec<String>)> = reach
        .iter()
        .map(|r| {
            let types: Vec<String> = r
                .types
                .iter()
                .filter(|t| visible.contains(t))
                .cloned()
                .collect();
            (r.party_id, types)
        })
        .filter(|(_, types)| !types.is_empty())
        .collect();
    if !searchable_query(query) || reach.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let dialect = db.dialect();
    let matches = lookup_identifier(db, query).await?;
    let mut by_type: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut matched: BTreeMap<(String, i64), String> = BTreeMap::new();
    for found in &matches {
        by_type
            .entry(found.subject.subject_type.clone())
            .or_default()
            .push(found.subject.subject_id);
        matched.insert(
            (found.subject.subject_type.clone(), found.subject.subject_id),
            found.kind.clone(),
        );
    }
    let fts = fts_query(dialect, query);
    let identifier_condition = |s: &mut Statement| {
        if by_type.is_empty() {
            s.push("1 = 0");
            return;
        }
        s.push("(");
        for (i, (subject_type, ids)) in by_type.iter().enumerate() {
            if i > 0 {
                s.push(" OR ");
            }
            s.push("(d.subject_type = ");
            s.bind(DbValue::Text(subject_type.clone()));
            s.push(" AND d.subject_id IN (");
            s.bind_list(ids.iter().map(|id| DbValue::Integer(*id)));
            s.push("))");
        }
        s.push(")");
    };

    let mut s = Statement::default();
    s.push("SELECT d.id AS id, p.party_id AS party_id, CAST(CASE WHEN ");
    identifier_condition(&mut s);
    s.push(" THEN 1 ELSE 0 END AS BIGINT) AS idm, ");
    match &fts {
        Some(expression) => match dialect {
            Dialect::Sqlite => {
                s.push(&format!(
                    "COALESCE((SELECT -bm25({FTS_TABLE}, 1.0, 0.4, 0.2, 0.1) FROM {FTS_TABLE} \
                     WHERE {FTS_TABLE} MATCH "
                ));
                s.bind(DbValue::Text(expression.clone()));
                s.push(&format!(" AND {FTS_TABLE}.rowid = d.id), 0)"));
            }
            Dialect::Postgres => {
                s.push("ts_rank(d.search_vector, to_tsquery('simple', ");
                s.bind(DbValue::Text(expression.clone()));
                s.push("))");
            }
        },
        None => s.push("0"),
    }
    s.push(
        " AS rk FROM wl_search_documents d JOIN wl_search_parties p \
         ON p.subject_type = d.subject_type AND p.subject_id = d.subject_id \
         WHERE d.portal_visible = ",
    );
    s.bind(DbValue::Bool(true));
    // The party filter: each party with the types allowed for it.
    s.push(" AND (");
    for (i, (party_id, types)) in reach.iter().enumerate() {
        if i > 0 {
            s.push(" OR ");
        }
        s.push("(p.party_id = ");
        s.bind(DbValue::Integer(*party_id));
        s.push(" AND d.subject_type IN (");
        s.bind_list(types.iter().map(|t| DbValue::Text(t.clone())));
        s.push("))");
    }
    s.push(") AND (");
    identifier_condition(&mut s);
    s.push(" OR ");
    match &fts {
        Some(expression) => match dialect {
            Dialect::Sqlite => {
                s.push(&format!(
                    "d.id IN (SELECT rowid FROM {FTS_TABLE} WHERE {FTS_TABLE} MATCH "
                ));
                s.bind(DbValue::Text(expression.clone()));
                s.push(")");
            }
            Dialect::Postgres => {
                s.push("d.search_vector @@ to_tsquery('simple', ");
                s.bind(DbValue::Text(expression.clone()));
                s.push(")");
            }
        },
        None => s.push("1 = 0"),
    }
    s.push(") ORDER BY idm DESC, rk DESC, d.updated_at DESC, d.id DESC LIMIT ");
    s.bind(DbValue::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));

    let rows = sql(s.text).bind_all(s.binds).fetch_all(db).await?;
    let mut picked: Vec<(i64, i64, bool)> = Vec::with_capacity(rows.len());
    for row in &rows {
        picked.push((
            row.try_get("id")?,
            row.try_get("party_id")?,
            row.try_get::<i64>("idm")? == 1,
        ));
    }
    if picked.is_empty() {
        return Ok(Vec::new());
    }
    let documents: BTreeMap<i64, SearchDocumentRow> = SearchDocumentRow::query()
        .where_in("id", picked.iter().map(|p| p.0).collect::<Vec<_>>())
        .get(db)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    Ok(picked
        .into_iter()
        .filter_map(|(id, party_id, identifier_match)| {
            let row = documents.get(&id)?;
            let searchable = registry::get(&row.subject_type)?;
            let matched = matched
                .get(&(row.subject_type.clone(), row.subject_id))
                .filter(|kind| kinds::PERSONAL.contains(&kind.as_str()))
                .map(|kind| mask(&normalize_identifier(kind, query)));
            Some(PortalHit {
                subject: Subject::new(&row.subject_type, row.subject_id),
                party_id,
                title: row.title.clone(),
                secondary: row.secondary.clone(),
                status: row.status.clone(),
                voided: row.voided,
                label: searchable.label().to_owned(),
                url: searchable.portal_url_of(party_id, row.subject_id),
                identifier_match,
                matched,
            })
        })
        .collect())
}
