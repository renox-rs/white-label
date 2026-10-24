//! Sorting by column and the CSV export of the report pages whose rows are
//! worked out in Rust (#526): the points liability and the acquisition
//! report.
//!
//! The standard grid (`renox::grid`) pages, sorts and exports the query of a
//! model. These reports are not the rows of one table: the liability is
//! worked out from the movements as at a date and the acquisition report from
//! the leads, sales and costs in the user's scope, grouped as chosen; so they
//! stay tables and get the two mechanisms the grid would give them that fit
//! a computed report: `?sort=key` (`-key` for descending) picked in the
//! report's own filter form, and a CSV export of the rows as shown. (The same
//! helper lives in wl-invoicing for its reports; the modules do not depend
//! on each other.)

use std::cmp::Ordering;

/// The value a report column sorts by.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub(crate) enum SortKey {
    /// An amount in fils, a count or a percentage in basis points.
    Number(i64),
    /// A name, compared without case.
    Text(String),
}

impl SortKey {
    /// A name, compared without case.
    pub(crate) fn text(text: &str) -> Self {
        SortKey::Text(text.to_lowercase())
    }
}

/// The column a report is sorted by, from `?sort=`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sort {
    /// The column's key.
    pub(crate) key: String,
    /// Highest (or Z) first.
    pub(crate) descending: bool,
}

impl Sort {
    /// `variance` or `-variance` when `variance` is one of `keys`; anything
    /// else keeps the report's own order.
    pub(crate) fn parse(raw: Option<&str>, keys: &[&str]) -> Option<Sort> {
        let raw = raw?.trim();
        let (descending, key) = match raw.strip_prefix('-') {
            Some(key) => (true, key),
            None => (false, raw),
        };
        keys.contains(&key).then(|| Sort {
            key: key.to_owned(),
            descending,
        })
    }

    /// The value of `?sort=` that asks for this order.
    pub(crate) fn param(&self) -> String {
        if self.descending {
            format!("-{}", self.key)
        } else {
            self.key.clone()
        }
    }

    /// Sorts `rows` by the value `key_of` gives for the column; equal rows
    /// keep their order.
    pub(crate) fn apply<T>(&self, rows: &mut [T], key_of: impl Fn(&T, &str) -> SortKey) {
        rows.sort_by(|a, b| {
            let order = key_of(a, &self.key)
                .partial_cmp(&key_of(b, &self.key))
                .unwrap_or(Ordering::Equal);
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
}

/// The options of a report's sort select: `("", own_order)` first, then
/// each column of `columns` (`(key, label)`) as `label, up` and
/// `label, down`.
pub(crate) fn sort_options(
    columns: &[(&str, String)],
    own_order: &str,
    up: &str,
    down: &str,
) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), own_order.to_owned())];
    for (key, label) in columns {
        options.push(((*key).to_owned(), format!("{label}, {up}")));
        options.push((format!("-{key}"), format!("{label}, {down}")));
    }
    options
}

/// Whether the query asks for the CSV export (`?export=csv`).
pub(crate) fn wants_csv(export: Option<&str>) -> bool {
    export.is_some_and(|e| e.trim().eq_ignore_ascii_case("csv"))
}

/// `head` and `rows` as a CSV file named `{name}.csv`, with a byte order
/// mark so Excel reads it as UTF-8.
pub(crate) fn csv(name: &str, head: &[String], rows: &[Vec<String>]) -> renox::Download {
    renox::Download::bytes(
        format!("{name}.csv"),
        "text/csv; charset=utf-8",
        csv_text(head, rows).into_bytes(),
    )
}

/// The text of [`csv`].
pub(crate) fn csv_text(head: &[String], rows: &[Vec<String>]) -> String {
    fn field(text: &str) -> String {
        if text.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    }
    let line = |cells: &[String]| {
        cells
            .iter()
            .map(|c| field(c.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut out = String::from("\u{feff}");
    out.push_str(&line(head));
    out.push('\n');
    for row in rows {
        out.push_str(&line(row));
        out.push('\n');
    }
    out
}

/// `query` (`a=b&c=d` pairs, empty values left out) with `export=csv`
/// added: the link of a report's export button.
pub(crate) fn export_link(path: &str, pairs: &[(&str, String)]) -> String {
    let encode = |text: &str| {
        text.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    let mut query: Vec<String> = pairs
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(name, value)| format!("{name}={}", encode(value.trim())))
        .collect();
    query.push("export=csv".to_owned());
    format!("{path}?{}", query.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sort_reads_a_known_column_and_its_direction() {
        let keys = ["name", "total"];
        assert_eq!(
            Sort::parse(Some("-total"), &keys),
            Some(Sort {
                key: "total".into(),
                descending: true
            })
        );
        assert_eq!(Sort::parse(Some("name"), &keys).unwrap().param(), "name");
        assert_eq!(
            Sort::parse(Some("-total"), &keys).unwrap().param(),
            "-total"
        );
        assert_eq!(Sort::parse(Some("other"), &keys), None);
        assert_eq!(Sort::parse(Some(""), &keys), None);
        assert_eq!(Sort::parse(None, &keys), None);
    }

    #[test]
    fn rows_sort_by_number_or_by_name_without_case() {
        let mut rows = vec![("beta", 3), ("Alpha", 10), ("gamma", 1)];
        let key = |row: &(&str, i64), key: &str| match key {
            "name" => SortKey::text(row.0),
            _ => SortKey::Number(row.1),
        };
        Sort::parse(Some("-total"), &["name", "total"])
            .unwrap()
            .apply(&mut rows, key);
        assert_eq!(rows, [("Alpha", 10), ("beta", 3), ("gamma", 1)]);
        Sort::parse(Some("total"), &["name", "total"])
            .unwrap()
            .apply(&mut rows, key);
        assert_eq!(rows[0], ("gamma", 1));
        Sort::parse(Some("name"), &["name", "total"])
            .unwrap()
            .apply(&mut rows, key);
        assert_eq!(rows[0], ("Alpha", 10));
    }

    #[test]
    fn the_csv_quotes_what_needs_it() {
        let text = csv_text(
            &["Name".into(), "Total".into()],
            &[vec!["Acme, LLC".into(), "1,200.00".into()]],
        );
        assert_eq!(text, "\u{feff}Name,Total\n\"Acme, LLC\",\"1,200.00\"\n");
        assert!(wants_csv(Some("csv")) && !wants_csv(Some("pdf")) && !wants_csv(None));
    }

    #[test]
    fn the_export_link_keeps_the_filters() {
        assert_eq!(
            export_link(
                "/budget-report",
                &[
                    ("entity", "3".into()),
                    ("month", String::new()),
                    ("sort", "-variance".into())
                ]
            ),
            "/budget-report?entity=3&sort=-variance&export=csv"
        );
    }

    #[test]
    fn the_sort_options_offer_each_column_both_ways() {
        let options = sort_options(&[("total", "Total".into())], "Report order", "up", "down");
        assert_eq!(
            options,
            [
                (String::new(), "Report order".to_owned()),
                ("total".to_owned(), "Total, up".to_owned()),
                ("-total".to_owned(), "Total, down".to_owned()),
            ]
        );
    }
}
