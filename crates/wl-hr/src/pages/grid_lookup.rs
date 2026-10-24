//! What the HR grids need to search and filter on names that live in another
//! module (the parties directory): a name is not in a table of this crate,
//! so the grid cannot `LIKE` it. The handler asks the directory which parties
//! the searched text names and hands the grid an SQL expression that is the
//! searched text for the rows of those parties and empty for the others; the
//! grid's own search then finds them (issue #527).

use std::collections::BTreeMap;

use renox::grid::GridRequest;
use renox::prelude::*;
use wl_kernel::parties::directory;

use crate::employee::Employee;

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
/// whose `column` is one of `ids`, else empty.
pub(crate) fn matched_sql(term: Option<&str>, column: &str, ids: &[i64]) -> String {
    let Some(term) = term else {
        return "''".to_owned();
    };
    if ids.is_empty() {
        return "''".to_owned();
    }
    let list = ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "CASE WHEN {{T}}.\"{column}\" IN ({list}) THEN {} ELSE '' END",
        literal(term)
    )
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

/// The SQL of the name search of a grid of employees: the searched text for
/// the employees whose party the directory finds by it.
pub(crate) async fn employee_match(state: &AppState, term: Option<&str>) -> String {
    let ids = party_ids(state, term).await;
    matched_sql(term, "party_id", &ids)
}

/// The SQL of the name search of a grid of records of an employee (column
/// `employee_id`): the searched text for the records of the employees whose
/// party the directory finds by it.
pub(crate) async fn of_employee_match(state: &AppState, term: Option<&str>) -> Result<String> {
    let parties = party_ids(state, term).await;
    if parties.is_empty() {
        return Ok("''".to_owned());
    }
    let employees: Vec<i64> = Employee::query()
        .where_(Employee::PARTY_ID.is_in(parties))
        .get(&state.db)
        .await?
        .iter()
        .map(|employee| employee.id)
        .collect();
    Ok(matched_sql(term, "employee_id", &employees))
}

/// The options of a select column on the distinct `values`, each in words
/// (`security_guard` reads *Security guard*), sorted by the words.
pub(crate) fn worded_options(values: impl IntoIterator<Item = String>) -> Vec<(String, String)> {
    let mut words: BTreeMap<String, String> = BTreeMap::new();
    for value in values {
        if value.is_empty() {
            continue;
        }
        let spaced = value.replace('_', " ");
        let mut chars = spaced.chars();
        let worded = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
        words.insert(value, worded);
    }
    let mut options: Vec<(String, String)> = words.into_iter().collect();
    options.sort_by(|a, b| a.1.cmp(&b.1));
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_has_no_placeholder_and_doubles_its_quotes() {
        assert_eq!(literal("O'Neil?"), "'O''Neil'");
    }

    #[test]
    fn the_match_is_the_term_for_the_ids_found() {
        assert_eq!(matched_sql(None, "party_id", &[1]), "''");
        assert_eq!(matched_sql(Some("x"), "party_id", &[]), "''");
        assert_eq!(
            matched_sql(Some("Sara"), "party_id", &[1, 2]),
            "CASE WHEN {T}.\"party_id\" IN (1, 2) THEN 'Sara' ELSE '' END"
        );
    }

    #[test]
    fn options_say_the_values_in_words_once() {
        let options = worded_options(vec![
            "security_guard".to_owned(),
            "driver".to_owned(),
            "security_guard".to_owned(),
            String::new(),
        ]);
        assert_eq!(
            options,
            vec![
                ("driver".to_owned(), "Driver".to_owned()),
                ("security_guard".to_owned(), "Security guard".to_owned()),
            ]
        );
    }
}
