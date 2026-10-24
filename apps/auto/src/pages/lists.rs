//! What the list pages of the app share: they are all `renox::grid`s (the
//! standard of the data lists, `docs/UX.md` §2.3), so the helpers here are the
//! SQL the columns of other tables need, the money columns in fils shown as
//! `AED 1,250.00`, and the texts a grid is built from.
//!
//! A grid is built from `Text`, a function from a key of the app's texts to
//! the words, so a test builds the same grid with the English texts and the
//! page with those of the user's language.

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use std::collections::HashMap;

use renox::serde_json::{Map, Value, json};
use serde::Serialize;

use super::text;
use crate::vehicle::Emirate;

/// The texts of the app (without the `auto.` prefix) in a language.
pub(crate) type Text<'a> = &'a dyn Fn(&str) -> String;

/// The texts of the app in `lang`.
pub(crate) fn texts_in(lang: &Lang) -> impl Fn(&str) -> String + '_ {
    move |key: &str| text(lang, key)
}

/// The English texts of the app, for the tests.
#[cfg(test)]
pub(crate) fn english_text(key: &str) -> String {
    crate::texts::english(&format!("auto.{key}"), &[])
}

/// The currency of every amount of the app (the amounts are stored in fils,
/// without a currency column: the workshop trades in dirhams).
pub(crate) const CURRENCY: &str = "AED";

/// A money column of amounts in fils, shown as `AED 1,250.00`: the grid shows
/// the currency of the `currency` value each row carries ([`with_currency`]).
pub(crate) fn money(key: &str, label: &str) -> Column {
    Column::money(key, label).currency_from("currency")
}

/// `page` with the currency each money column reads and, for each row, the
/// values `extra` works out (the names of its parties, which the app asks the
/// parties module for, never from its tables: a `custom` column draws them).
pub(crate) fn with_currency<M: Model + Serialize>(
    page: GridPage<M>,
    extra: impl Fn(&M) -> Value,
) -> GridPage<M> {
    page.extend(|row| {
        let mut values = match extra(row) {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        values.insert("currency".to_owned(), json!(CURRENCY));
        Value::Object(values)
    })
}

/// The names of the parties `ids` in `lang`, each asked once.
pub(crate) async fn party_names(
    state: &AppState,
    lang: &Lang,
    ids: impl IntoIterator<Item = i64>,
) -> HashMap<i64, String> {
    let mut names = HashMap::new();
    for id in ids {
        if let std::collections::hash_map::Entry::Vacant(slot) = names.entry(id) {
            slot.insert(super::party_name(state, &lang.locale, id).await);
        }
    }
    names
}

/// The number of a record as people read it: `JOB-00007`, the prefix and the
/// id padded with zeros to `width` digits (a longer id is not cut).
pub(crate) fn numbered_sql(prefix: &str, width: usize) -> String {
    let zeros = "0".repeat(width);
    let limit = format!("1{zeros}");
    format!(
        "'{prefix}' || CASE WHEN {{T}}.\"id\" < {limit} THEN \
         SUBSTR('{zeros}' || CAST({{T}}.\"id\" AS TEXT), LENGTH(CAST({{T}}.\"id\" AS TEXT)) + 1) \
         ELSE CAST({{T}}.\"id\" AS TEXT) END"
    )
}

/// The name of the user in the model's column `fk`.
pub(crate) fn user_sql(fk: &str) -> String {
    format!("(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = {{T}}.\"{fk}\")")
}

/// The plate as people read it, `Dubai A 12345`, from the columns of `table`
/// (a quoted table name, or `{T}`): the emirate by its name on a plate.
fn plate_expr(table: &str) -> String {
    let emirate = format!("{table}.\"plate_emirate\"");
    let mut name = format!("CASE {emirate}");
    for emirate_value in Emirate::FORM_ORDER {
        name.push_str(&format!(
            " WHEN '{}' THEN '{}'",
            emirate_value.as_str(),
            emirate_value.label()
        ));
    }
    name.push_str(&format!(" ELSE {emirate} END"));
    format!("{name} || ' ' || {table}.\"plate_code\" || ' ' || {table}.\"plate_number\"")
}

/// The plate of a vehicle row, for the vehicles list.
pub(crate) fn own_plate_sql() -> String {
    format!("({})", plate_expr("{T}"))
}

/// The plate of the vehicle in the model's column `fk`: `Dubai A 12345`.
pub(crate) fn plate_sql(fk: &str) -> String {
    format!(
        "(SELECT {} FROM \"auto_vehicles\" WHERE \"auto_vehicles\".\"id\" = {{T}}.\"{fk}\")",
        plate_expr("\"auto_vehicles\"")
    )
}

/// `Toyota Hilux 2022` of the vehicle in the model's column `fk`.
pub(crate) fn vehicle_sql(fk: &str) -> String {
    format!(
        "(SELECT \"auto_vehicles\".\"make\" || ' ' || \"auto_vehicles\".\"model_name\" || \
         CASE WHEN \"auto_vehicles\".\"model_year\" IS NULL THEN '' \
         ELSE ' ' || CAST(\"auto_vehicles\".\"model_year\" AS TEXT) END \
         FROM \"auto_vehicles\" WHERE \"auto_vehicles\".\"id\" = {{T}}.\"{fk}\")"
    )
}

/// The baseline every list of the app shares (#523, #557): 25 rows a page,
/// the user's search, filters, sort and page size remembered, and the
/// advanced filter. The export menu is added by the page, for the roles that
/// may export ([`exporting`]).
pub(crate) fn baseline(grid: Grid) -> Grid {
    grid.per_page(25).remember().advanced_filter()
}

/// `grid` with its export menu when `can_export`.
pub(crate) fn exporting(grid: Grid, can_export: bool) -> Grid {
    if can_export { grid.exports() } else { grid }
}

/// The file of the export the request asks for (`?export=csv`), if any. A
/// user who may not export gets 403: [`Grid::export`] answers any request
/// that names a format, whether the menu is shown or not. Custom columns
/// (the party names) are never exported.
///
/// # Errors
///
/// 403 for a user who may not export; the database's errors.
pub(crate) async fn export_file<M: Model + Serialize>(
    grid: &Grid,
    can_export: bool,
    query: renox::db::Query<M>,
    request: &GridRequest,
) -> Result<Option<Response>> {
    if request.param("export").is_none() {
        return Ok(None);
    }
    if !can_export {
        return Err(Error::Forbidden);
    }
    grid.export(query, request).await
}

/// The options of a select column: each stored value with the words `key`
/// gives (`jobs.status` and `checked_in` read `jobs.status.checked_in`).
pub(crate) fn options<'a>(
    t: Text<'_>,
    key: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String)> {
    values
        .into_iter()
        .map(|value| (value.to_owned(), t(&format!("{key}.{value}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_padded_to_its_width() {
        let sql = numbered_sql("JOB-", 5);
        assert!(sql.starts_with("'JOB-' || CASE WHEN {T}.\"id\" < 100000"));
        assert!(sql.contains("SUBSTR('00000' ||"));
    }

    #[test]
    fn the_plate_names_every_emirate() {
        let sql = plate_sql("vehicle_id");
        for emirate in Emirate::FORM_ORDER {
            assert!(sql.contains(&format!(
                "WHEN '{}' THEN '{}'",
                emirate.as_str(),
                emirate.label()
            )));
        }
    }
}
