//! What the grids of the app need to search and filter on names that live in
//! another module (the parties directory, the workforce): the names are not
//! in a table of this app, so the grid cannot `LIKE` them. The handler asks
//! the directory which records the searched text names and hands the grid an
//! SQL expression that is the searched text for those rows and empty for the
//! others; the grid's own search then finds them. A filter on such a name is
//! a `select` column on the id, with the names as its options.

use std::collections::BTreeMap;

use renox::grid::GridRequest;
use renox::prelude::*;
use wl_kernel::parties::directory;

/// The text the grid `grid` searches for: the request's `search`, or the one
/// the grid remembered for this person when the request carries no state
/// (as `Grid::remember` reads it).
pub(crate) fn searched(request: &GridRequest, session: &Session, grid: &str) -> Option<String> {
    let remembered: Option<Vec<(String, String)>> = session.get(&format!("_grid_q.{grid}"));
    let params = match (request.param("state"), remembered) {
        (None, Some(saved)) => saved,
        _ => request
            .param("search")
            .map(|value| vec![("search".to_owned(), value.to_owned())])
            .unwrap_or_default(),
    };
    params
        .into_iter()
        .find(|(name, _)| name == "search")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// `text` as a quoted SQL literal: quotes doubled; no `?` (a placeholder) and
/// no backslash, at most 200 characters.
pub(crate) fn literal(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !matches!(c, '?' | '\\'))
        .take(200)
        .collect();
    format!("'{}'", clean.replace('\'', "''"))
}

/// The SQL of a hidden searchable column: the searched `term` for the rows
/// whose any of `columns` is one of `ids`, else empty.
pub(crate) fn matched_sql(term: Option<&str>, columns: &[&str], ids: &[i64]) -> String {
    let Some(term) = term else {
        return "''".to_owned();
    };
    if ids.is_empty() || columns.is_empty() {
        return "''".to_owned();
    }
    let list = ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let test = columns
        .iter()
        .map(|column| format!("{{T}}.\"{column}\" IN ({list})"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("CASE WHEN {test} THEN {} ELSE '' END", literal(term))
}

/// The parties the directory finds by `term` (name or code), at most 200.
pub(crate) async fn party_ids(state: &AppState, term: Option<&str>) -> Vec<i64> {
    let Some(term) = term else {
        return Vec::new();
    };
    directory(state)
        .search(&state.db, term, 200)
        .await
        .unwrap_or_default()
        .iter()
        .map(|party| party.id.0)
        .collect()
}

/// The SQL of the search column on parties: the searched text for the rows
/// whose any of `columns` names a party the directory finds by it.
pub(crate) async fn party_match(state: &AppState, term: Option<&str>, columns: &[&str]) -> String {
    let ids = party_ids(state, term).await;
    matched_sql(term, columns, &ids)
}

/// The options of a filter on the parties `ids`: the id and the name, sorted
/// by the name.
pub(crate) async fn party_options(
    state: &AppState,
    locale: &str,
    ids: impl IntoIterator<Item = i64>,
) -> Vec<(String, String)> {
    let mut names = BTreeMap::new();
    for id in ids {
        if let std::collections::btree_map::Entry::Vacant(slot) = names.entry(id) {
            let name = directory(state)
                .display_name(&state.db, wl_kernel::PartyId(id), locale)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| format!("#{id}"));
            slot.insert(name);
        }
    }
    let mut options: Vec<(String, String)> = names
        .into_iter()
        .map(|(id, name)| (id.to_string(), name))
        .collect();
    options.sort_by(|a, b| a.1.cmp(&b.1));
    options
}

/// The SQL of a select column whose value is not stored in the row: the key
/// `keys` gives the row's `column`, else empty (`CASE … WHEN id THEN 'key'`).
pub(crate) fn keyed_sql(column: &str, keys: &BTreeMap<i64, String>) -> String {
    if keys.is_empty() {
        return "''".to_owned();
    }
    let arms = keys
        .iter()
        .map(|(id, key)| format!("WHEN {id} THEN {}", literal(key)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("CASE {{T}}.\"{column}\" {arms} ELSE '' END")
}

/// A filter on a value worked out per row in Rust (`labels`: the row's
/// `column` value to its label, e.g. a booking to its branch): the SQL of the
/// select column and its options, one key (`k1`, `k2` …) per distinct label,
/// sorted by the label.
pub(crate) fn labelled(column: &str, labels: &BTreeMap<i64, String>) -> (String, Vec<(String, String)>) {
    let mut distinct: Vec<&String> = labels.values().collect();
    distinct.sort();
    distinct.dedup();
    let options: Vec<(String, String)> = distinct
        .iter()
        .enumerate()
        .map(|(i, label)| (format!("k{}", i + 1), (*label).clone()))
        .collect();
    let keys: BTreeMap<i64, String> = labels
        .iter()
        .filter_map(|(id, label)| {
            options
                .iter()
                .find(|(_, shown)| shown == label)
                .map(|(key, _)| (*id, key.clone()))
        })
        .collect();
    (keyed_sql(column, &keys), options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_labelled_column_keys_each_label_once() {
        let labels = BTreeMap::from([
            (1, "Marina".to_owned()),
            (2, "Al Barsha".to_owned()),
            (3, "Marina".to_owned()),
        ]);
        let (sql, options) = labelled("id", &labels);
        assert_eq!(
            options,
            vec![
                ("k1".to_owned(), "Al Barsha".to_owned()),
                ("k2".to_owned(), "Marina".to_owned())
            ]
        );
        assert_eq!(
            sql,
            "CASE {T}.\"id\" WHEN 1 THEN 'k2' WHEN 2 THEN 'k1' WHEN 3 THEN 'k2' ELSE '' END"
        );
    }

    #[test]
    fn a_literal_has_no_placeholder_and_doubles_its_quotes() {
        assert_eq!(literal("O'Neil?"), "'O''Neil'");
    }

    #[test]
    fn the_match_is_the_term_for_the_ids_found() {
        assert_eq!(matched_sql(None, &["a"], &[1]), "''");
        assert_eq!(matched_sql(Some("x"), &["a"], &[]), "''");
        assert_eq!(
            matched_sql(Some("Sara"), &["a", "b"], &[1, 2]),
            "CASE WHEN {T}.\"a\" IN (1, 2) OR {T}.\"b\" IN (1, 2) THEN 'Sara' ELSE '' END"
        );
    }

    #[test]
    fn a_keyed_column_names_each_row() {
        let keys = BTreeMap::from([(4, "b1".to_owned())]);
        assert_eq!(
            keyed_sql("booking_id", &keys),
            "CASE {T}.\"booking_id\" WHEN 4 THEN 'b1' ELSE '' END"
        );
    }
}
