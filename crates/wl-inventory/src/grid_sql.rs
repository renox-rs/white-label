//! SQL for the grid columns whose values are worked out in Rust (the stock
//! of an item at a store, the name of a supplier in the parties directory):
//! the handler hands the grid a `CASE` on the row's id with the value of each
//! listed row, so that the grid sorts, filters, sums and exports them like a
//! stored column (issue #528).

use std::collections::BTreeMap;

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

/// The SQL of a number column: `values` gives the number of the row whose
/// `column` is the key, 0 for the others (a `BIGINT` on both databases).
pub(crate) fn numbers_sql(column: &str, values: &BTreeMap<i64, i64>) -> String {
    if values.is_empty() {
        return "0".to_owned();
    }
    let arms = values
        .iter()
        .map(|(id, value)| format!("WHEN {id} THEN {value}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("CAST((CASE {{T}}.\"{column}\" {arms} ELSE 0 END) AS BIGINT)")
}

/// The SQL of a text column: `words` gives the text of the row whose
/// `column` is the key, empty for the others.
pub(crate) fn words_sql(column: &str, words: &BTreeMap<i64, String>) -> String {
    if words.is_empty() {
        return "''".to_owned();
    }
    let arms = words
        .iter()
        .map(|(id, text)| format!("WHEN {id} THEN {}", literal(text)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("(CASE {{T}}.\"{column}\" {arms} ELSE '' END)")
}

/// The options of a select column on the ids of `names`, by name.
pub(crate) fn options_of(names: &BTreeMap<i64, String>) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = names
        .iter()
        .map(|(id, name)| (id.to_string(), name.clone()))
        .collect();
    options.sort_by(|a, b| a.1.cmp(&b.1));
    options
}

/// The SQL of a select column on an id column (`CAST(... AS TEXT)`), so that
/// it compares with the options of [`options_of`].
pub(crate) fn id_key_sql(column: &str) -> String {
    format!("CAST({{T}}.\"{column}\" AS TEXT)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_has_no_placeholder_and_doubles_its_quotes() {
        assert_eq!(literal("O'Neil?"), "'O''Neil'");
    }

    #[test]
    fn a_number_column_names_each_row_and_defaults_to_zero() {
        assert_eq!(numbers_sql("id", &BTreeMap::new()), "0");
        let values = BTreeMap::from([(4, 1500), (7, -2)]);
        assert_eq!(
            numbers_sql("id", &values),
            "CAST((CASE {T}.\"id\" WHEN 4 THEN 1500 WHEN 7 THEN -2 ELSE 0 END) AS BIGINT)"
        );
    }

    #[test]
    fn a_text_column_names_each_row() {
        let words = BTreeMap::from([(2, "Gulf Foods".to_owned())]);
        assert_eq!(
            words_sql("supplier_party_id", &words),
            "(CASE {T}.\"supplier_party_id\" WHEN 2 THEN 'Gulf Foods' ELSE '' END)"
        );
        assert_eq!(words_sql("id", &BTreeMap::new()), "''");
    }

    #[test]
    fn options_come_by_name() {
        let names = BTreeMap::from([(1, "Zeta".to_owned()), (2, "Alpha".to_owned())]);
        assert_eq!(
            options_of(&names),
            vec![
                ("2".to_owned(), "Alpha".to_owned()),
                ("1".to_owned(), "Zeta".to_owned())
            ]
        );
    }
}
