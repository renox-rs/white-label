//! The receivables ageing page (wl-invoicing §6, A15):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /receivables/ageing` | `wl_invoicing.ageing` | `receivables.view` |
//!
//! One row per party and currency: the balance in each bucket of the
//! `.ageing_buckets` setting, the disputed part and the total. `?entity=`
//! limits it to a legal entity. The template is compiled in and can be
//! replaced by the app (`resources/views/wl-invoicing/ageing.html`).

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

use crate::ageing::{self, AgeingReport};
use crate::allocate::amount_text;
use crate::config_of;
use crate::permissions;
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
}

/// `GET /receivables/ageing`: the balances by days past due.
async fn index(
    State(state): State<AppState>,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
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
    Ok(View::typed(IndexPage {
        tx,
        legend: report.labels.join(", "),
        rows: lines(&report),
        entities,
        entity: entity.map(|e| e.to_string()).unwrap_or_default(),
        on: report.on.format("%Y-%m-%d").to_string(),
    }))
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
            }
            .entity_id()
        };
        assert_eq!(of(None), None);
        assert_eq!(of(Some("")), None);
        assert_eq!(of(Some("x")), None);
        assert_eq!(of(Some("3")), Some(3));
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
