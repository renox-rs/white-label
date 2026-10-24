//! The receivables ageing page (wl-invoicing §6, A15):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /receivables/ageing` | `wl_invoicing.ageing` | `receivables.view` |
//!
//! One row per party and currency: the balance in each bucket of the
//! `.ageing_buckets` setting, the disputed part and the total. `?entity=`
//! limits it to a legal entity; `?sort=total` (`-total` for the largest
//! first; also `party`, `currency`, `disputed` and `bucket1`, `bucket2`… for
//! the buckets in order) sorts the rows, and `?export=csv` downloads them
//! ([`crate::report_table`], #525). The template is compiled in and can be
//! replaced by the app (`resources/views/wl-invoicing/ageing.html`).

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

use crate::ageing::{self, AgeingReport, AgeingRow};
use crate::allocate::amount_text;
use crate::config_of;
use crate::permissions;
use crate::report_table::{self, Sort, SortKey};
use crate::texts;

/// The texts a template reads as `tx['ageing.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's ageing route, behind `receivables.view`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/receivables/ageing", index)
        .name("wl_invoicing.ageing")
        .require_permission(permissions::RECEIVABLES_VIEW)
}

/// The query of the page.
#[derive(Deserialize, Default, Debug)]
pub(crate) struct Filter {
    /// The legal entity id; empty or missing for all.
    entity: Option<String>,
    /// The column the rows are sorted by, `-` first for descending.
    sort: Option<String>,
    /// `csv`: the rows as a file.
    export: Option<String>,
}

/// The columns the rows sort by: the party, the currency, each of the
/// `buckets` (`bucket1`, `bucket2`…), the disputed part and the total.
fn sort_keys(buckets: usize) -> Vec<String> {
    let mut keys = vec!["party".to_owned(), "currency".to_owned()];
    keys.extend((1..=buckets).map(|n| format!("bucket{n}")));
    keys.extend(["disputed".to_owned(), "total".to_owned()]);
    keys
}

/// The value of `row` that the column `key` sorts by.
fn sort_key(row: &AgeingRow, key: &str) -> SortKey {
    match key {
        "party" => SortKey::text(&row.party),
        "currency" => SortKey::text(&row.currency),
        "disputed" => SortKey::Number(row.disputed),
        "total" => SortKey::Number(row.total),
        bucket => {
            let n: usize = bucket
                .strip_prefix("bucket")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            SortKey::Number(
                n.checked_sub(1)
                    .and_then(|i| row.buckets.get(i))
                    .copied()
                    .unwrap_or(0),
            )
        }
    }
}

/// `report` with its rows in the order `raw` (`?sort=`) asks for; the
/// order of the report (by party) otherwise.
pub(crate) fn sorted(
    mut report: AgeingReport,
    raw: Option<&str>,
) -> (AgeingReport, Option<Sort>) {
    let keys = sort_keys(report.labels.len());
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let sort = Sort::parse(raw, &keys);
    if let Some(sort) = &sort {
        sort.apply(&mut report.rows, sort_key);
    }
    (report, sort)
}

impl Filter {
    /// The entity id asked for, if any.
    fn entity_id(&self) -> Option<i64> {
        self.entity.as_deref().and_then(|e| e.trim().parse().ok())
    }
}

/// One bucket of a row.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    label: String,
    amount: String,
}

/// One row as the page shows it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    party: String,
    currency: String,
    /// The amount of each bucket.
    cells: Vec<Cell>,
    disputed: String,
    total: String,
}

/// The rows of `report` as the page shows them.
pub(crate) fn lines(report: &AgeingReport) -> Vec<Line> {
    report
        .rows
        .iter()
        .map(|row| Line {
            party: row.party.clone(),
            currency: row.currency.clone(),
            cells: row
                .buckets
                .iter()
                .zip(&report.labels)
                .map(|(v, label)| Cell {
                    label: label.clone(),
                    amount: amount_text(&row.currency, *v),
                })
                .collect(),
            disputed: amount_text(&row.currency, row.disputed),
            total: amount_text(&row.currency, row.total),
        })
        .collect()
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/ageing.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    /// The buckets: `0-30, 31-60, 61-90, 90+`.
    legend: String,
    rows: Vec<Line>,
    /// The legal entities to filter by, `(id or "", label)`.
    entities: Vec<(String, String)>,
    /// The selected entity, `""` for all.
    entity: String,
    /// The day the days past due are counted to.
    on: String,
    /// The headings of the table: party, currency, the buckets, disputed,
    /// total.
    head: Vec<String>,
    /// The orders the sort select offers, and the one chosen.
    sorts: Vec<(String, String)>,
    sort: String,
    /// The link of the CSV export, with the page's filters.
    export_url: String,
}

/// `GET /receivables/ageing`: the balances by days past due, or the rows as
/// a CSV file (`?export=csv`).
async fn index(
    State(state): State<AppState>,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    let tx = texts::all(&lang);
    let mut entities = vec![(
        String::new(),
        texts::t(&lang, "wl_invoicing.ageing.all_entities"),
    )];
    entities.extend(
        LegalEntity::query()
            .order_by(LegalEntity::CODE)
            .get(&state.db)
            .await?
            .into_iter()
            .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en))),
    );
    let entity = filter.entity_id();
    let edges = config_of(&state).ageing_buckets.clone();
    let report = ageing::report(&state.db, &edges, entity, today()).await?;
    let (report, sort) = sorted(report, filter.sort.as_deref());
    let rows = lines(&report);
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    let mut columns: Vec<(String, String)> = vec![
        ("party".to_owned(), t("ageing.party")),
        ("currency".to_owned(), t("ageing.currency")),
    ];
    columns.extend(
        report
            .labels
            .iter()
            .enumerate()
            .map(|(i, label)| (format!("bucket{}", i + 1), label.clone())),
    );
    columns.push(("disputed".to_owned(), t("ageing.disputed")));
    columns.push(("total".to_owned(), t("ageing.total")));
    let head: Vec<String> = columns.iter().map(|(_, label)| label.clone()).collect();
    if report_table::wants_csv(filter.export.as_deref()) {
        let lines: Vec<Vec<String>> = rows
            .iter()
            .map(|line| {
                let mut cells = vec![line.party.clone(), line.currency.clone()];
                cells.extend(line.cells.iter().map(|c| c.amount.clone()));
                cells.push(line.disputed.clone());
                cells.push(line.total.clone());
                cells
            })
            .collect();
        return Ok(report_table::csv("receivables-ageing", &head, &lines).into_response());
    }
    let sort_columns: Vec<(&str, String)> = columns
        .iter()
        .map(|(key, label)| (key.as_str(), label.clone()))
        .collect();
    let sorts = report_table::sort_options(
        &sort_columns,
        &t("report.own_order"),
        &t("report.up"),
        &t("report.down"),
    );
    let entity = entity.map(|e| e.to_string()).unwrap_or_default();
    let sort = sort.as_ref().map(Sort::param).unwrap_or_default();
    let export_url = report_table::export_link(
        "/receivables/ageing",
        &[("entity", entity.clone()), ("sort", sort.clone())],
    );
    Ok(View::typed(IndexPage {
        tx,
        legend: report.labels.join(", "),
        rows,
        entities,
        entity,
        on: report.on.format("%Y-%m-%d").to_string(),
        head,
        sorts,
        sort,
        export_url,
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ageing::AgeingRow;
    use renox::chrono::NaiveDate;

    #[test]
    fn the_entity_filter_reads_an_id_or_means_all() {
        let of = |e: Option<&str>| {
            Filter {
                entity: e.map(str::to_owned),
                ..Filter::default()
            }
            .entity_id()
        };
        assert_eq!(of(None), None);
        assert_eq!(of(Some("")), None);
        assert_eq!(of(Some("x")), None);
        assert_eq!(of(Some("3")), Some(3));
    }

    #[test]
    fn the_rows_sort_by_a_bucket_the_party_or_the_total() {
        let row = |party: &str, buckets: Vec<i64>, total| AgeingRow {
            party_id: 1,
            party: party.into(),
            currency: "AED".into(),
            buckets,
            disputed: 0,
            total,
        };
        let report = AgeingReport {
            on: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            labels: vec!["0-30".into(), "31+".into()],
            rows: vec![
                row("Acme", vec![100, 900], 1_000),
                row("Blue Dunes", vec![500, 0], 500),
                row("Coral", vec![0, 2_000], 2_000),
            ],
        };
        let parties = |(report, _): (AgeingReport, Option<Sort>)| -> Vec<String> {
            report.rows.into_iter().map(|r| r.party).collect()
        };
        let by = |key: &str| parties(sorted(report.clone(), Some(key)));
        assert_eq!(by("-total"), ["Coral", "Acme", "Blue Dunes"]);
        assert_eq!(by("-bucket1"), ["Blue Dunes", "Acme", "Coral"]);
        assert_eq!(by("bucket2"), ["Blue Dunes", "Acme", "Coral"]);
        assert_eq!(by("-party"), ["Coral", "Blue Dunes", "Acme"]);
        // A bucket the setting does not have keeps the report's order.
        let (same, sort) = sorted(report.clone(), Some("bucket3"));
        assert!(sort.is_none());
        assert_eq!(same, report);
    }

    #[test]
    fn the_amounts_are_written_in_the_currency() {
        let report = AgeingReport {
            on: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            labels: vec!["0-30".into(), "31+".into()],
            rows: vec![AgeingRow {
                party_id: 1,
                party: "Acme".into(),
                currency: "AED".into(),
                buckets: vec![380_000, 0],
                disputed: 120_000,
                total: 500_000,
            }],
        };
        let lines = lines(&report);
        let cells: Vec<(&str, &str)> = lines[0]
            .cells
            .iter()
            .map(|c| (c.label.as_str(), c.amount.as_str()))
            .collect();
        assert_eq!(cells, [("0-30", "AED 3,800.00"), ("31+", "AED 0.00")]);
        assert_eq!(lines[0].disputed, "AED 1,200.00");
        assert_eq!(lines[0].total, "AED 5,000.00");
    }
}
