//! The points liability report page (wl-sales §5.9 step 6; BL-SAL-24), behind
//! `loyalty.view` (the accountant has it).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /loyalty/liability?as_at=2026-12-31` | `wl_sales.loyalty_liability` | `loyalty.view` |
//!
//! `?sort=points` (`-points` for the most first; also `brand`, `entity`,
//! `age`, `expiry_month`, `value`) orders the lines and `?export=csv`
//! downloads them as shown (#526, [`super::report_table`]).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::documents::today;

use super::report_table::{self, Sort, SortKey};
use crate::liability::{self, LiabilityRow};
use crate::notices::aed;
use crate::{permissions, texts};

/// The texts a template reads as `tx['liability.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes, behind `loyalty.view`.
pub fn routes() -> Routes {
    Routes::new()
        .get("/loyalty/liability", show)
        .name("wl_sales.loyalty_liability")
        .require_permission(permissions::LOYALTY_VIEW)
}

/// The package text `key` (without the `wl_sales.` prefix).
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_sales.{key}"))
}

/// The columns the lines sort by.
const SORT_KEYS: [&str; 6] = ["brand", "entity", "age", "expiry_month", "points", "value"];

/// The value of `row` that the column `key` sorts by; the ages in their own
/// order, youngest first.
fn sort_key(row: &LiabilityRow, key: &str) -> SortKey {
    match key {
        "brand" => SortKey::text(&row.brand),
        "entity" => SortKey::Number(row.entity.unwrap_or(0)),
        "age" => SortKey::Number(match row.age {
            "0_90" => 0,
            "91_180" => 1,
            "181_365" => 2,
            _ => 3,
        }),
        "expiry_month" => SortKey::text(&row.expiry_month),
        "points" => SortKey::Number(row.points),
        _ => SortKey::Number(row.value),
    }
}

/// `rows` in the order `raw` (`?sort=`) asks for, else as the report gives
/// them.
pub(crate) fn sorted(rows: &mut [LiabilityRow], raw: Option<&str>) -> Option<Sort> {
    let sort = Sort::parse(raw, &SORT_KEYS);
    if let Some(sort) = &sort {
        sort.apply(rows, sort_key);
    }
    sort
}

/// One line as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct Line {
    /// Tells the line from the others.
    pub key: String,
    /// The brand, or `(client)`.
    pub brand: String,
    /// The legal entity's id, or empty.
    pub entity: String,
    /// The age bucket in words.
    pub age: String,
    /// The expiry month, `2027-06`.
    pub expiry_month: String,
    /// The points.
    pub points: i64,
    /// Their value, `AED 1,200.00`.
    pub value: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/liability.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    as_at: String,
    lines: Vec<Line>,
    total_points: i64,
    total_value: String,
    /// The orders the sort select offers, and the one chosen.
    sorts: Vec<(String, String)>,
    sort: String,
    /// The link of the CSV export, with the date and the order.
    export_url: String,
}

/// `GET /loyalty/liability`: the report as at the `as_at` date (today by
/// default), or its lines as a CSV file (`?export=csv`).
async fn show(
    State(db): State<Db>,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Response> {
    let as_at = match query
        .get("as_at")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        None => today(),
        Some(text_) => text_.parse::<NaiveDate>().map_err(|_| {
            let mut errors = Errors::new();
            errors.add("as_at", text(&lang, "errors.date"));
            Error::Validation(ValidationError::new(errors).with_input(&query))
        })?,
    };
    let mut report = liability::report(&db, as_at).await?;
    let sort = sorted(&mut report.rows, query.get("sort").map(String::as_str));
    let lines: Vec<Line> = report
        .rows
        .iter()
        .map(|row| Line {
            key: format!(
                "{}|{}|{}|{}",
                row.brand,
                row.entity.map(|e| e.to_string()).unwrap_or_default(),
                row.age,
                row.expiry_month
            ),
            brand: if row.brand.is_empty() {
                text(&lang, "liability.own_brand")
            } else {
                row.brand.clone()
            },
            entity: row.entity.map(|e| e.to_string()).unwrap_or_default(),
            age: text(&lang, &format!("liability.age.{}", row.age)),
            expiry_month: row.expiry_month.clone(),
            points: row.points,
            value: format!("AED {}", aed(row.value)),
        })
        .collect();
    let columns: Vec<(&str, String)> = SORT_KEYS
        .iter()
        .map(|key| (*key, text(&lang, &format!("liability.{key}"))))
        .collect();
    let head: Vec<String> = columns.iter().map(|(_, label)| label.clone()).collect();
    if report_table::wants_csv(query.get("export").map(String::as_str)) {
        let rows: Vec<Vec<String>> = lines
            .iter()
            .map(|line| {
                vec![
                    line.brand.clone(),
                    line.entity.clone(),
                    line.age.clone(),
                    line.expiry_month.clone(),
                    line.points.to_string(),
                    line.value.clone(),
                ]
            })
            .collect();
        let file = report_table::csv(&format!("points-liability-{as_at}"), &head, &rows);
        return Ok(file.into_response());
    }
    let sorts = report_table::sort_options(
        &columns,
        &text(&lang, "report.own_order"),
        &text(&lang, "report.up"),
        &text(&lang, "report.down"),
    );
    let sort = sort.as_ref().map(Sort::param).unwrap_or_default();
    let export_url = report_table::export_link(
        "/loyalty/liability",
        &[("as_at", as_at.to_string()), ("sort", sort.clone())],
    );
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        as_at: as_at.to_string(),
        lines,
        total_points: report.points,
        total_value: format!("AED {}", aed(report.value)),
        sorts,
        sort,
        export_url,
    })
    .into_response())
}
