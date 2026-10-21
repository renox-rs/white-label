//! The points liability report page (wl-sales §5.9 step 6; BL-SAL-24), behind
//! `loyalty.view` (the accountant has it).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /loyalty/liability?as_at=2026-12-31` | `wl_sales.loyalty_liability` | `loyalty.view` |

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::validation::{Errors, ValidationError};
use serde::Serialize;
use wl_kernel::documents::today;

use crate::liability;
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
}

/// `GET /loyalty/liability`: the report as at the `as_at` date (today by
/// default).
async fn show(
    State(db): State<Db>,
    lang: Lang,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<View> {
    let as_at = match query.get("as_at").map(|v| v.trim()).filter(|v| !v.is_empty()) {
        None => today(),
        Some(text_) => text_.parse::<NaiveDate>().map_err(|_| {
            let mut errors = Errors::new();
            errors.add("as_at", text(&lang, "errors.date"));
            Error::Validation(ValidationError::new(errors).with_input(&query))
        })?,
    };
    let report = liability::report(&db, as_at).await?;
    let lines = report
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
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        as_at: as_at.to_string(),
        lines,
        total_points: report.points,
        total_value: format!("AED {}", aed(report.value)),
    }))
}
