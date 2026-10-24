//! The acquisition report page and its CSV (wl-sales §5.8 step 5, issue
//! #140).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /sales/acquisition-report` | `wl_sales.acquisition_report` | `acquisition.reports.view`, globally or in a branch |
//! | `GET /sales/acquisition-report.csv` | `wl_sales.acquisition_report.csv` | the same |
//!
//! The query picks the grouping (`?group=source|campaign|channel|brand|
//! branch|month`) and the filters (`source` and `campaign` by code,
//! `channel`, `brand`, `branch` as `type:id`, `entity` by id, `month` as
//! `2026-04`). What a user sees follows their scope ([`Viewer`]): a global
//! role sees every branch, a role given in a branch sees that branch only. The
//! routes carry no permission guard of their own, since the guard counts
//! global roles only; a user with no role that grants the permission
//! anywhere gets a 403. The CSV has the numbers of the page. `?sort=cac`
//! (`-cac` for the highest first; any column: `group`, `leads`, `won`,
//! `conversion`, `customers`, `value`, `cost`, `cost_linked`, `cost_manual`,
//! `cost_per_lead`, `cac`, `roas`) orders the rows on the page and in the CSV
//! (#526); a ratio that cannot be worked out ("—") sorts as the lowest.

use std::collections::BTreeMap;

use renox::auth::permissions::Scope;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::Subject;

use crate::campaign::{Campaign, MarketingChannel};
use crate::config::config_of;
use crate::lead_source::LeadSource;
use crate::permissions;
use super::report_table::{self, Sort, SortKey};
use crate::report::{self, Cells, Facts, Filter, GroupBy, Report, Row, Viewer};
use crate::texts;

/// The texts a template reads as `tx['report.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The routes of the report.
pub fn routes() -> Routes {
    Routes::new()
        .get("/sales/acquisition-report", index)
        .name("wl_sales.acquisition_report")
        .get("/sales/acquisition-report.csv", csv)
        .name("wl_sales.acquisition_report.csv")
}

/// The query of the page.
#[derive(Deserialize, Serialize, Default, Debug, Clone)]
pub struct ReportQuery {
    /// The grouping; default the source.
    #[serde(default)]
    pub group: Option<String>,
    /// A lead source code.
    #[serde(default)]
    pub source: Option<String>,
    /// A campaign code.
    #[serde(default)]
    pub campaign: Option<String>,
    /// A marketing channel, as stored (`meta`).
    #[serde(default)]
    pub channel: Option<String>,
    /// A brand.
    #[serde(default)]
    pub brand: Option<String>,
    /// A branch, `type:id`.
    #[serde(default)]
    pub branch: Option<String>,
    /// A legal entity id.
    #[serde(default)]
    pub entity: Option<String>,
    /// A month, `2026-04`.
    #[serde(default)]
    pub month: Option<String>,
    /// The column the rows are sorted by, `-` first for descending.
    #[serde(default)]
    pub sort: Option<String>,
}

/// The columns the rows sort by.
const SORT_KEYS: [&str; 12] = [
    "group",
    "leads",
    "won",
    "conversion",
    "customers",
    "value",
    "cost",
    "cost_linked",
    "cost_manual",
    "cost_per_lead",
    "cac",
    "roas",
];

/// The value of `row` (named `label`) that the column `key` sorts by; a
/// cost the viewer may not see, or a ratio with nothing to divide by, is the
/// lowest.
fn sort_key(row: &Row, label: &str, key: &str) -> SortKey {
    let none = i64::MIN;
    let cost_known = !row.cost_hidden && row.cost.is_some();
    let number = match key {
        "group" => return SortKey::text(label),
        "leads" => row.leads,
        "won" => row.won,
        "conversion" => row.conversion_tenths().unwrap_or(none),
        "customers" => row.customers,
        "value" => row.value,
        "cost" if cost_known => row.cost.unwrap_or(none),
        "cost_linked" if cost_known => row.cost_linked,
        "cost_manual" if cost_known => row.cost_manual,
        "cost_per_lead" => row.cost_per_lead().unwrap_or(none),
        "cac" => row.cac().unwrap_or(none),
        "roas" => row.roas_hundredths().unwrap_or(none),
        _ => none,
    };
    SortKey::Number(number)
}

/// The rows of `report` and their `labels` in the order of `sort`.
fn sorted(
    mut report: Report,
    labels: Vec<String>,
    sort: Option<&Sort>,
) -> (Report, Vec<String>) {
    let Some(sort) = sort else {
        return (report, labels);
    };
    let mut order: Vec<usize> = (0..report.rows.len()).collect();
    sort.apply(&mut order, |&i, key| {
        sort_key(&report.rows[i], labels.get(i).map_or("", String::as_str), key)
    });
    let mut rows: Vec<Option<Row>> = report.rows.drain(..).map(Some).collect();
    let mut names: Vec<Option<String>> = labels.into_iter().map(Some).collect();
    let mut ordered_labels = Vec::with_capacity(order.len());
    for i in order {
        if let Some(row) = rows.get_mut(i).and_then(Option::take) {
            report.rows.push(row);
            ordered_labels.push(names.get_mut(i).and_then(Option::take).unwrap_or_default());
        }
    }
    (report, ordered_labels)
}

/// `text` trimmed, `None` when blank.
fn given(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// `2026-04` as the year and month.
fn parse_month(text: &str) -> Option<(i32, u32)> {
    let (year, month) = text.trim().split_once('-')?;
    let (year, month) = (year.parse().ok()?, month.parse().ok()?);
    (1..=12).contains(&month).then_some((year, month))
}

impl ReportQuery {
    /// The grouping asked for.
    pub fn group_by(&self) -> GroupBy {
        given(&self.group)
            .and_then(|g| GroupBy::parse(&g))
            .unwrap_or_default()
    }

    /// The filter asked for. A source or campaign code that matches nothing
    /// filters everything out (id 0).
    pub async fn filter(&self, db: &Db) -> Result<Filter> {
        let mut filter = Filter::default();
        if let Some(code) = given(&self.source) {
            filter.source = Some(
                LeadSource::by_code(db, &LeadSource::normalize_code(&code))
                    .await?
                    .map_or(0, |s| s.id),
            );
        }
        if let Some(code) = given(&self.campaign) {
            filter.campaign = Some(Campaign::by_code(db, &code).await?.map_or(0, |c| c.id));
        }
        filter.channel = given(&self.channel);
        filter.brand = given(&self.brand);
        filter.branch = given(&self.branch).and_then(|b| b.parse::<Subject>().ok());
        filter.entity = given(&self.entity).and_then(|e| e.parse().ok());
        if let Some((year, month)) = given(&self.month).as_deref().and_then(parse_month) {
            filter = filter.month(year, month);
        }
        Ok(filter)
    }
}

/// The branches `user` sees the report in, from the roles given there.
fn viewer_of(user: &AuthUser, facts: &Facts) -> Viewer {
    if user.has_permission(permissions::ACQUISITION_REPORTS_VIEW) {
        return Viewer::all();
    }
    Viewer::scoped(
        facts
            .branches()
            .into_iter()
            .filter(|branch| {
                user.has_permission_in(
                    permissions::ACQUISITION_REPORTS_VIEW,
                    &Scope::new(&branch.subject_type, branch.subject_id),
                )
            })
            .collect(),
    )
}

/// The report for `query` as `user` sees it, with the names of its rows.
async fn build(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    query: &ReportQuery,
) -> Result<(Report, Vec<String>)> {
    let facts = report::load(&state.db, &config_of(state)).await?;
    let viewer = viewer_of(user, &facts);
    if viewer.is_blind() {
        return Err(Error::Forbidden);
    }
    let filter = query.filter(&state.db).await?;
    let report = report::compute(&facts, &filter, query.group_by(), &viewer);
    let mut labels = Vec::new();
    for row in &report.rows {
        labels.push(report::label_of(&state.db, &row.key, &lang.locale).await?);
    }
    let sort = Sort::parse(query.sort.as_deref(), &SORT_KEYS);
    Ok(sorted(report, labels, sort.as_ref()))
}

/// One row as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct RowView {
    /// The name of the group.
    pub label: String,
    /// The numbers.
    pub cells: Cells,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-sales/acquisition_report.html")]
pub(crate) struct ReportPage {
    tx: Texts,
    rows: Vec<RowView>,
    total: Cells,
    head: Vec<String>,
    groups: Vec<(String, String)>,
    group: String,
    sources: Vec<(String, String)>,
    source: String,
    channels: Vec<(String, String)>,
    channel: String,
    entities: Vec<(String, String)>,
    entity: String,
    campaign: String,
    brand: String,
    branch: String,
    month: String,
    csv_query: String,
    /// The orders the sort select offers, and the one chosen.
    sorts: Vec<(String, String)>,
    sort: String,
}

/// `GET /sales/acquisition-report`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<ReportQuery>,
) -> Result<View> {
    let (report, labels) = build(&state, &user, &lang, &query).await?;
    let t = |key: &str| texts::t(&lang, &format!("wl_sales.{key}"));
    let head = [
        report.by.name(),
        "report.leads",
        "report.won",
        "report.conversion",
        "report.customers",
        "report.value",
        "report.cost",
        "report.cost_linked",
        "report.cost_manual",
        "report.cost_per_lead",
        "report.cac",
        "report.roas",
    ]
    .iter()
    .map(|key| {
        if key.starts_with("report.") {
            t(key)
        } else {
            t(&format!("report.by.{key}"))
        }
    })
    .collect::<Vec<String>>();
    let rows = report
        .rows
        .iter()
        .zip(labels)
        .map(|(row, label)| RowView {
            label,
            cells: row.cells(),
        })
        .collect();
    let groups = GroupBy::ALL
        .iter()
        .map(|g| (g.name().to_owned(), t(&format!("report.by.{}", g.name()))))
        .collect();
    let mut sources = vec![(String::new(), t("report.all"))];
    sources.extend(
        LeadSource::ordered(&state.db)
            .await?
            .into_iter()
            .map(|s| (s.code.clone(), s.name(&lang.locale).to_owned())),
    );
    let mut channels = vec![(String::new(), t("report.all"))];
    channels.extend(MarketingChannel::ALL.iter().map(|c| {
        (
            c.as_str().to_owned(),
            t(&format!("campaigns.channel.{}", c.as_str())),
        )
    }));
    let mut entities = vec![(String::new(), t("report.all"))];
    entities.extend(
        LegalEntity::query()
            .order_by(LegalEntity::CODE)
            .get(&state.db)
            .await?
            .into_iter()
            .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en))),
    );
    let csv_query = serde_urlencoded_pairs(&query);
    let sort_columns: Vec<(&str, String)> = SORT_KEYS
        .iter()
        .zip(head.iter())
        .map(|(key, label)| (*key, label.clone()))
        .collect();
    let sorts = report_table::sort_options(
        &sort_columns,
        &t("report.own_order"),
        &t("report.up"),
        &t("report.down"),
    );
    let sort = Sort::parse(query.sort.as_deref(), &SORT_KEYS)
        .as_ref()
        .map(Sort::param)
        .unwrap_or_default();
    Ok(View::typed(ReportPage {
        tx: texts::all(&lang),
        rows,
        total: report.total.cells(),
        head,
        groups,
        group: query.group_by().name().to_owned(),
        sources,
        source: given(&query.source).unwrap_or_default(),
        channels,
        channel: given(&query.channel).unwrap_or_default(),
        entities,
        entity: given(&query.entity).unwrap_or_default(),
        campaign: given(&query.campaign).unwrap_or_default(),
        brand: given(&query.brand).unwrap_or_default(),
        branch: given(&query.branch).unwrap_or_default(),
        month: given(&query.month).unwrap_or_default(),
        csv_query,
        sorts,
        sort,
    }))
}

/// The query as `a=b&c=d`, for the CSV link.
fn serde_urlencoded_pairs(query: &ReportQuery) -> String {
    let pairs = [
        ("group", &query.group),
        ("source", &query.source),
        ("campaign", &query.campaign),
        ("channel", &query.channel),
        ("brand", &query.brand),
        ("branch", &query.branch),
        ("entity", &query.entity),
        ("month", &query.month),
        ("sort", &query.sort),
    ];
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
    pairs
        .iter()
        .filter_map(|(name, value)| given(value).map(|v| format!("{name}={}", encode(&v))))
        .collect::<Vec<_>>()
        .join("&")
}

/// `GET /sales/acquisition-report.csv`.
async fn csv(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(query): Query<ReportQuery>,
) -> Result<renox::Download> {
    let (report, labels) = build(&state, &user, &lang, &query).await?;
    let total_label = texts::t(&lang, "wl_sales.report.total");
    let body = report::to_csv(&report, &labels, &total_label);
    Ok(renox::Download::bytes(
        "acquisition-report.csv",
        "text/csv; charset=utf-8",
        body.into_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::GroupKey;

    fn row(value: &str, leads: i64, won: i64, cost: Option<i64>) -> Row {
        Row {
            key: GroupKey {
                by: GroupBy::Campaign,
                value: Some(value.to_owned()),
            },
            leads,
            won,
            customers: won,
            value: 0,
            cost,
            cost_linked: 0,
            cost_manual: cost.unwrap_or(0),
            cost_hidden: false,
        }
    }

    fn report() -> (Report, Vec<String>) {
        let rows = vec![
            row("1", 50, 10, Some(500_000)),
            row("2", 10, 9, None),
            row("3", 30, 3, Some(30_000)),
        ];
        let total = row("", 90, 22, Some(530_000));
        let labels = vec!["SPRING26".to_owned(), "autumn".to_owned(), "Winter".to_owned()];
        (
            Report {
                by: GroupBy::Campaign,
                rows,
                total,
            },
            labels,
        )
    }

    fn by(key: &str) -> Vec<String> {
        let (report, labels) = report();
        let sort = Sort::parse(Some(key), &SORT_KEYS);
        assert!(sort.is_some(), "{key} is a column");
        let (report, labels) = sorted(report, labels, sort.as_ref());
        assert_eq!(report.rows.len(), labels.len());
        // Each label stays with its row.
        for (row, label) in report.rows.iter().zip(&labels) {
            let expected = match row.key.value.as_deref() {
                Some("1") => "SPRING26",
                Some("2") => "autumn",
                _ => "Winter",
            };
            assert_eq!(label, expected);
        }
        labels
    }

    #[test]
    fn the_rows_sort_by_any_column_with_their_labels() {
        assert_eq!(by("-leads"), ["SPRING26", "Winter", "autumn"]);
        assert_eq!(by("group"), ["autumn", "SPRING26", "Winter"]);
        assert_eq!(by("-conversion"), ["autumn", "SPRING26", "Winter"]);
        // No cost: no cost per lead, the lowest.
        assert_eq!(by("cost_per_lead"), ["autumn", "Winter", "SPRING26"]);
        let (report, labels) = report();
        let (_, same) = sorted(report, labels.clone(), None);
        assert_eq!(same, labels);
    }
}
