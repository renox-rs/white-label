//! Reading the index for a person (BL-AGT-02, wl-kernel §5.8, decision 0006
//! "How search works" 1–2): identifiers first, then full-text matches, grouped
//! by type, filtered by the type's permission and the user's scopes **inside
//! the query**, before anything is counted or limited, so neither a page, a
//! count nor a redirect can reveal a record the user may not open.
//!
//! One page costs a constant number of statements whatever the number of
//! results: the identifier lookup ([`lookup_identifier`], at most three), one
//! ranked and counted statement over the documents (window functions, on
//! SQLite and PostgreSQL alike) and one read of the documents it picked.
//!
//! Ranking: an identifier match, then the full-text rank (FTS5 `bm25` on
//! SQLite, `ts_rank` on PostgreSQL, over the index Renox keeps on
//! `wl_search_documents`), then the most recently updated.
//!
//! A word of the query matches a word of the document that starts with it,
//! or a word with the same Arabic–Latin match key (`renox::text::match_key`,
//! the `forms` column), so "Mohammed" finds "محمد" and the reverse.

use std::collections::{BTreeMap, BTreeSet};

use renox::db::{DbValue, Dialect, sql};
use renox::prelude::*;
use serde::Serialize;

use super::access::Access;
use super::document::normalize_identifier;
use super::index::lookup_identifier;
use super::registry;
use super::{SearchDocumentRow, kinds};
use crate::subject::Subject;

/// The shortest query that is searched; a shorter one gets a hint.
pub const MIN_QUERY_CHARS: usize = 2;

/// Results per type on the grouped page.
pub const PER_TYPE: u64 = 5;

/// Results per page of one type (*Show all*).
pub const PER_PAGE: u64 = 25;

/// The most words of a query that are searched.
const MAX_WORDS: usize = 8;

/// The full-text table Renox keeps on SQLite for `wl_search_documents`.
pub(super) const FTS_TABLE: &str = "\"wl_search_documents_search\"";

/// One record found.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    /// The record.
    pub subject: Subject,
    /// What it is called, e.g. `INV-PGP-2026-0004`.
    pub title: String,
    /// The line under the title.
    pub secondary: String,
    /// Its status, if the module gives one.
    pub status: Option<String>,
    /// Whether it was voided.
    pub voided: bool,
    /// The address of its page.
    pub url: String,
    /// Whether an identifier of the record equals the query.
    pub identifier_match: bool,
    /// A sensitive identifier that matched, masked (`•••• 4567`).
    pub matched: Option<String>,
    /// When its document was last written.
    pub updated_at: Option<DateTime>,
}

/// The results of one type.
#[derive(Debug, Clone, Serialize)]
pub struct SearchGroup {
    /// The type, e.g. `invoicing.invoice`.
    pub subject_type: String,
    /// The group's name, e.g. `Invoices`.
    pub label: String,
    /// How many records of the type match, among those the user may see.
    pub total: u64,
    /// The records of this page, best first.
    pub hits: Vec<SearchHit>,
}

/// The grouped results of a query.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchResults {
    /// The query as typed, trimmed.
    pub query: String,
    /// The groups, the one with the best match first.
    pub groups: Vec<SearchGroup>,
    /// The page to open at once: exactly one record the user may see has an
    /// identifier equal to the query.
    pub open: Option<String>,
}

/// Whether `text` is long enough to search ([`MIN_QUERY_CHARS`]).
pub fn searchable_query(text: &str) -> bool {
    text.trim().chars().count() >= MIN_QUERY_CHARS
}

/// Searches every type `access` may see: up to `per_type` records of each,
/// with each type's total. Empty when the query is too short or the user may
/// see no type.
pub async fn search(db: &Db, access: &Access, text: &str, per_type: u64) -> Result<SearchResults> {
    let query = text.trim().to_owned();
    if !searchable_query(&query) {
        return Ok(SearchResults {
            query,
            ..Default::default()
        });
    }
    let rows = run(db, access, &query, None, 0, per_type).await?;
    let open = {
        let hits: Vec<&Picked> = rows.picked.iter().filter(|p| p.identifier).collect();
        match (rows.identifier_hits, hits.first()) {
            (1, Some(picked)) => rows.hit(picked).map(|hit| hit.url),
            _ => None,
        }
    };
    let mut groups: Vec<(i64, SearchGroup)> = Vec::new();
    for picked in &rows.picked {
        let Some(hit) = rows.hit(picked) else { continue };
        match groups
            .iter_mut()
            .find(|(_, g)| g.subject_type == picked.subject_type)
        {
            Some((_, group)) => group.hits.push(hit),
            None => groups.push((
                picked.position,
                SearchGroup {
                    label: label_of(&picked.subject_type),
                    subject_type: picked.subject_type.clone(),
                    total: picked.total,
                    hits: vec![hit],
                },
            )),
        }
    }
    groups.sort_by_key(|(position, _)| *position);
    Ok(SearchResults {
        query,
        groups: groups.into_iter().map(|(_, group)| group).collect(),
        open,
    })
}

/// Page `page` (from 1) of the records of `subject_type` matching `text`,
/// `per_page` at a time, with the type's total. `None` when the user may
/// not see the type, or the query is too short.
pub async fn search_type(
    db: &Db,
    access: &Access,
    text: &str,
    subject_type: &str,
    page: u64,
    per_page: u64,
) -> Result<Option<SearchGroup>> {
    let query = text.trim();
    let Some(searchable) = registry::get(subject_type) else {
        return Ok(None);
    };
    if !searchable_query(query) || !access.holds(searchable.permission()) {
        return Ok(None);
    }
    let page = page.max(1);
    let rows = run(
        db,
        access,
        query,
        Some(subject_type),
        (page - 1) * per_page,
        page * per_page,
    )
    .await?;
    let total = rows.picked.first().map_or(0, |p| p.total);
    let total = if total == 0 && page > 1 {
        // Past the last page: count again from the first.
        run(db, access, query, Some(subject_type), 0, 1)
            .await?
            .picked
            .first()
            .map_or(0, |p| p.total)
    } else {
        total
    };
    Ok(Some(SearchGroup {
        subject_type: subject_type.to_owned(),
        label: searchable.label().to_owned(),
        total,
        hits: rows.picked.iter().filter_map(|p| rows.hit(p)).collect(),
    }))
}

/// The group name of `subject_type`.
fn label_of(subject_type: &str) -> String {
    registry::get(subject_type).map_or_else(
        || crate::subject::words_of(subject_type, 0).replace(" #0", ""),
        |searchable| searchable.label().to_owned(),
    )
}

/// A document the ranked statement picked.
struct Picked {
    id: i64,
    subject_type: String,
    identifier: bool,
    total: u64,
    position: i64,
}

/// What [`run`] read.
struct Ran {
    picked: Vec<Picked>,
    identifier_hits: i64,
    documents: BTreeMap<i64, SearchDocumentRow>,
    /// The identifier kind that matched each record.
    matched: BTreeMap<(String, i64), String>,
    query: String,
}

impl Ran {
    fn hit(&self, picked: &Picked) -> Option<SearchHit> {
        let row = self.documents.get(&picked.id)?;
        let kind = self
            .matched
            .get(&(row.subject_type.clone(), row.subject_id));
        let matched = kind
            .filter(|kind| kinds::PERSONAL.contains(&kind.as_str()))
            .map(|kind| mask(&normalize_identifier(kind, &self.query)));
        let url = registry::get(&row.subject_type).map_or_else(
            || format!("/related/{}:{}", row.subject_type, row.subject_id),
            |searchable| searchable.url_of(row.subject_id),
        );
        Some(SearchHit {
            subject: Subject::new(&row.subject_type, row.subject_id),
            title: row.title.clone(),
            secondary: row.secondary.clone(),
            status: row.status.clone(),
            voided: row.voided,
            url,
            identifier_match: picked.identifier,
            matched,
            updated_at: row.updated_at,
        })
    }
}

/// `value` with all but its last four characters hidden: `•••• 4567`.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    format!("•••• {tail}")
}

/// A statement being built: its text and its values, in the order of their
/// `?`.
#[derive(Default)]
pub(super) struct Statement {
    pub(super) text: String,
    pub(super) binds: Vec<DbValue>,
}

impl Statement {
    pub(super) fn push(&mut self, text: &str) {
        self.text.push_str(text);
    }

    pub(super) fn bind(&mut self, value: DbValue) {
        self.text.push('?');
        self.binds.push(value);
    }

    pub(super) fn bind_list(&mut self, values: impl IntoIterator<Item = DbValue>) {
        let mut first = true;
        for value in values {
            if !first {
                self.text.push_str(", ");
            }
            first = false;
            self.text.push('?');
            self.binds.push(value);
        }
    }
}

/// The ranked, filtered and counted statement: the documents ranked
/// `from + 1 ..= to` of each type (of `only` when given).
async fn run(
    db: &Db,
    access: &Access,
    query: &str,
    only: Option<&str>,
    from: u64,
    to: u64,
) -> Result<Ran> {
    let dialect = db.dialect();
    let matches = lookup_identifier(db, query).await?;
    let mut by_type: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut matched = BTreeMap::new();
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
    let types: Vec<registry::Searchable> = access
        .searchable_types()
        .into_iter()
        .filter(|searchable| only.is_none_or(|t| t == searchable.subject_type()))
        .collect();
    let empty = Ran {
        picked: Vec::new(),
        identifier_hits: 0,
        documents: BTreeMap::new(),
        matched: BTreeMap::new(),
        query: query.to_owned(),
    };
    if types.is_empty() {
        return Ok(empty);
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
    s.push(
        "SELECT y.id, y.subject_type, y.idm, CAST(y.total AS BIGINT) AS total, \
         CAST(y.id_hits AS BIGINT) AS id_hits, CAST(y.pos AS BIGINT) AS pos FROM (\
         SELECT x.id, x.subject_type, x.idm, \
         ROW_NUMBER() OVER (PARTITION BY x.subject_type \
         ORDER BY x.idm DESC, x.rk DESC, x.updated_at DESC, x.id DESC) AS rn, \
         ROW_NUMBER() OVER (ORDER BY x.idm DESC, x.rk DESC, x.updated_at DESC, x.id DESC) AS pos, \
         COUNT(*) OVER (PARTITION BY x.subject_type) AS total, \
         SUM(x.idm) OVER () AS id_hits FROM (\
         SELECT d.id AS id, d.subject_type AS subject_type, d.updated_at AS updated_at, \
         CAST(CASE WHEN ",
    );
    identifier_condition(&mut s);
    s.push(" THEN 1 ELSE 0 END AS BIGINT) AS idm, ");
    match &fts {
        Some(expression) => {
            match dialect {
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
            }
        }
        None => s.push("0"),
    }
    s.push(" AS rk FROM wl_search_documents d WHERE (");
    // The access filter: the types the user may see, each within its reach.
    for (i, searchable) in types.iter().enumerate() {
        if i > 0 {
            s.push(" OR ");
        }
        s.push("(d.subject_type = ");
        s.bind(DbValue::Text(searchable.subject_type().to_owned()));
        if let Some(reach) = access
            .reach(searchable.permission())
            .filter(|reach| !reach.is_all())
        {
            s.push(" AND ");
            push_scopes(&mut s, reach);
        }
        // A record that needs a permission of its own (a confidential
        // ticket) is seen where the user holds that permission too.
        if !searchable.record_permissions().is_empty() {
            s.push(" AND (d.permission = ");
            s.bind(DbValue::Text(searchable.permission().to_owned()));
            for extra in searchable.record_permissions() {
                let Some(reach) = access.reach(extra) else {
                    continue;
                };
                s.push(" OR (d.permission = ");
                s.bind(DbValue::Text(extra.clone()));
                if !reach.is_all() {
                    s.push(" AND ");
                    push_scopes(&mut s, reach);
                }
                s.push(")");
            }
            s.push(")");
        }
        s.push(")");
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
    s.push(")) x) y WHERE y.rn > ");
    s.bind(DbValue::Integer(i64::try_from(from).unwrap_or(i64::MAX)));
    s.push(" AND y.rn <= ");
    s.bind(DbValue::Integer(i64::try_from(to).unwrap_or(i64::MAX)));
    s.push(" ORDER BY y.pos");

    let rows = sql(s.text).bind_all(s.binds).fetch_all(db).await?;
    let mut picked = Vec::with_capacity(rows.len());
    let mut identifier_hits = 0;
    for row in &rows {
        identifier_hits = row.try_get::<i64>("id_hits")?;
        picked.push(Picked {
            id: row.try_get("id")?,
            subject_type: row.try_get("subject_type")?,
            identifier: row.try_get::<i64>("idm")? == 1,
            total: u64::try_from(row.try_get::<i64>("total")?).unwrap_or(0),
            position: row.try_get("pos")?,
        });
    }
    if picked.is_empty() {
        return Ok(empty);
    }
    let documents = SearchDocumentRow::query()
        .where_in("id", picked.iter().map(|p| p.id).collect::<Vec<_>>())
        .get(db)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    Ok(Ran {
        picked,
        identifier_hits,
        documents,
        matched,
        query: query.to_owned(),
    })
}

/// The condition that a document's scope is one of `reach`'s scopes
/// (never true for an empty list), in parentheses.
fn push_scopes(s: &mut Statement, reach: &crate::scopes::Reach) {
    let scopes = reach.scopes();
    s.push("(");
    if scopes.is_empty() {
        s.push("1 = 0");
    }
    for (j, scope) in scopes.iter().enumerate() {
        if j > 0 {
            s.push(" OR ");
        }
        s.push("(d.scope_type = ");
        s.bind(DbValue::Text(scope.kind().to_owned()));
        s.push(" AND d.scope_id = ");
        s.bind(DbValue::Text(scope.id().to_owned()));
        s.push(")");
    }
    s.push(")");
}

/// The full-text query for `text`: each word as the start of a word, or its
/// Arabic–Latin match key; every word required. Only letters and digits
/// reach the database, so the input cannot carry search syntax. `None` when
/// the text has no word.
pub(super) fn fts_query(dialect: Dialect, text: &str) -> Option<String> {
    let normalized = renox::text::normalize_arabic(text);
    let mut seen = BTreeSet::new();
    let words: Vec<String> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.chars().take(64).collect::<String>().to_lowercase())
        .filter(|word| seen.insert(word.clone()))
        .take(MAX_WORDS)
        .collect();
    if words.is_empty() {
        return None;
    }
    let terms: Vec<String> = words
        .iter()
        .map(|word| {
            let key = renox::text::match_key(word);
            let alternatives: Vec<String> = std::iter::once(word.clone())
                .chain(
                    (key.chars().count() >= 2 && key != *word && key.chars().all(char::is_alphanumeric))
                        .then_some(key),
                )
                .collect();
            match dialect {
                Dialect::Sqlite => {
                    let parts: Vec<String> =
                        alternatives.iter().enumerate().map(|(i, a)| {
                            // The word as the start of a word; its key whole.
                            if i == 0 { format!("\"{a}\"*") } else { format!("\"{a}\"") }
                        }).collect();
                    format!("({})", parts.join(" OR "))
                }
                Dialect::Postgres => {
                    let parts: Vec<String> = alternatives
                        .iter()
                        .enumerate()
                        .map(|(i, a)| if i == 0 { format!("{a}:*") } else { a.clone() })
                        .collect();
                    format!("({})", parts.join(" | "))
                }
            }
        })
        .collect();
    Some(match dialect {
        Dialect::Sqlite => terms.join(" AND "),
        Dialect::Postgres => terms.join(" & "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_reaches_the_index_as_words_only() {
        let query = fts_query(Dialect::Sqlite, "inv\"* OR pgp").unwrap();
        let parts: Vec<&str> = query.split(" AND ").collect();
        assert_eq!(parts.len(), 3, "{query}");
        assert!(parts[0].starts_with("(\"inv\"*"), "{query}");
        assert!(parts[1].starts_with("(\"or\"*"), "{query}");
        assert!(parts[2].starts_with("(\"pgp\"*"), "{query}");
        assert_eq!(fts_query(Dialect::Sqlite, " -- "), None);
    }

    #[test]
    fn a_name_also_matches_its_arabic_latin_key() {
        let query = fts_query(Dialect::Postgres, "Mohammed").unwrap();
        assert_eq!(query, "(mohammed:* | mhmd)");
    }

    #[test]
    fn a_sensitive_value_shows_its_last_four_characters() {
        assert_eq!(mask("784199012345671"), "•••• 5671");
        assert_eq!(mask("12"), "•••• 12");
    }
}
