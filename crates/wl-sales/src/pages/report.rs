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
//! anywhere gets a 403. The CSV has the numbers of the page.

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
use crate::report::{self, Cells, Facts, Filter, GroupBy, Report, Viewer};
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
    Ok((report, labels))
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
    .collect();
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
