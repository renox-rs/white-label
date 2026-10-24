//! The budget vs actual page (wl-invoicing §5.15 step 3, `FIN.ACL.22`):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /budget-report` | `wl_invoicing.budget_report` | `budgets.view` |
//!
//! `?entity=` limits it to one legal entity (empty: all the user's entities),
//! `?month=2027-03` picks the month (default: this month) and `?ytd=1` shows
//! the year to date up to it. A user sees the entities whose scope grants
//! `budgets.view` (a global role grants all of them); a requested entity
//! outside the scope is 403. Without `drawings.approve` the drawings show
//! their total only; with it, the lines, the payees and the *self-approved*
//! flag are listed too. `?sort=variance` (`-variance` for the largest first;
//! also `category`, `budget`, `actual`, `percent`) orders the categories
//! inside each section, and `?export=csv` downloads the table as shown
//! ([`crate::report_table`], #525). The template is compiled in and can be
//! replaced by the app (`resources/views/wl-invoicing/budget_report.html`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

use crate::allocate::amount_text;
use crate::budget::month_start;
use crate::budget_category::BudgetKind;
use crate::budget_lines::parse_month;
use crate::budget_report::{self, Report, ReportQuery, Row, percent_bp, unfavourable};
use crate::config_of;
use crate::permissions;
use crate::report_table::{self, Sort, SortKey};
use crate::texts;

/// The texts a template reads as `tx['budget.title']`.
type Texts = BTreeMap<&'static str, String>;

/// The module's report route; the handler checks the permission against the
/// user's entity scope.
pub fn routes() -> Routes {
    Routes::new()
        .get("/budget-report", index)
        .name("wl_invoicing.budget_report")
}

/// The query of the page.
#[derive(Deserialize, Default, Debug)]
pub(crate) struct Filter {
    /// The legal entity id; empty or missing for all.
    entity: Option<String>,
    /// The month, `2027-03`; empty or missing for this month.
    month: Option<String>,
    /// Ticked: the year to date.
    ytd: Option<String>,
    /// The column the categories are sorted by, `-` first for descending.
    sort: Option<String>,
    /// `csv`: the table as a file.
    export: Option<String>,
}

/// The columns the categories can be sorted by.
const SORT_KEYS: [&str; 5] = ["category", "budget", "actual", "variance", "percent"];

/// The value of `row` that the column `key` sorts by.
fn sort_key(row: &Row, key: &str) -> SortKey {
    match key {
        "category" => SortKey::text(&row.name),
        "budget" => SortKey::Number(row.budget),
        "actual" => SortKey::Number(row.actual),
        "variance" => SortKey::Number(row.actual - row.budget),
        // A category with no budget has no percentage: it comes first.
        _ => SortKey::Number(
            percent_bp(row.actual - row.budget, row.budget).unwrap_or(i64::MIN),
        ),
    }
}

/// `report` with the categories of each section in the order of `sort`.
pub(crate) fn sorted(mut report: Report, sort: Option<&Sort>) -> Report {
    if let Some(sort) = sort {
        sort.apply(&mut report.income, sort_key);
        sort.apply(&mut report.expense, sort_key);
        sort.apply(&mut report.drawings, sort_key);
    }
    report
}

impl Filter {
    /// The entity id asked for, if any.
    fn entity_id(&self) -> Option<i64> {
        self.entity.as_deref().and_then(|e| e.trim().parse().ok())
    }

    /// The month asked for, else the month of `today`.
    fn month(&self, today: NaiveDate) -> NaiveDate {
        self.month
            .as_deref()
            .and_then(parse_month)
            .unwrap_or_else(|| month_start(today))
    }

    /// Whether the year to date is asked for.
    fn ytd(&self) -> bool {
        self.ytd
            .as_deref()
            .is_some_and(|v| !v.is_empty() && v != "0" && v != "false")
    }

    /// The order asked for, if any.
    fn sort(&self) -> Option<Sort> {
        Sort::parse(self.sort.as_deref(), &SORT_KEYS)
    }
}

/// `+500.00`, `-12.00`, `0.00`: a variance in AED.
pub(crate) fn signed_text(fils: i64) -> String {
    let plain = amount_text("AED", fils.abs());
    let plain = plain.trim_start_matches("AED ");
    match fils {
        0 => plain.to_owned(),
        f if f > 0 => format!("+{plain}"),
        _ => format!("-{plain}"),
    }
}

/// `5.00%`, `-25.50%`; `—` against a budget of 0.
pub(crate) fn percent_text(bp: Option<i64>) -> String {
    match bp {
        None => "\u{2014}".to_owned(),
        Some(bp) => {
            let sign = if bp < 0 { "-" } else { "" };
            let bp = bp.abs();
            format!("{sign}{}.{:02}%", bp / 100, bp % 100)
        }
    }
}

/// An amount as the report writes it: `20,000.00`.
fn plain(fils: i64) -> String {
    amount_text("AED", fils)
        .trim_start_matches("AED ")
        .to_owned()
}

/// The page of the document an item of actual comes from, as far as it has
/// one.
pub(crate) fn source_link(source: &wl_kernel::Subject) -> String {
    match source.subject_type.as_str() {
        "invoicing.supplier_bill" => format!("/supplier-bills/{}", source.subject_id),
        "invoicing.invoice" => format!("/invoices/{}", source.subject_id),
        "payments.payment" => format!("/payments/{}", source.subject_id),
        _ => String::new(),
    }
}

/// How a row of the table looks.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Look {
    /// A heading with no figures.
    Heading,
    /// A category.
    Line,
    /// A total or a result.
    Total,
}

/// One row of the table.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowView {
    look: Look,
    name: String,
    budget: String,
    actual: String,
    variance: String,
    percent: String,
    unfavourable: bool,
}

impl RowView {
    fn heading(name: &str) -> Self {
        RowView {
            look: Look::Heading,
            name: name.to_owned(),
            budget: String::new(),
            actual: String::new(),
            variance: String::new(),
            percent: String::new(),
            unfavourable: false,
        }
    }

    fn figures(look: Look, name: &str, kind: BudgetKind, budget: i64, actual: i64) -> Self {
        let variance = actual - budget;
        RowView {
            look,
            name: name.to_owned(),
            budget: plain(budget),
            actual: plain(actual),
            variance: signed_text(variance),
            percent: percent_text(percent_bp(variance, budget)),
            unfavourable: unfavourable(kind, variance),
        }
    }

    fn of(row: &Row) -> Self {
        Self::figures(Look::Line, &row.name, row.kind, row.budget, row.actual)
    }
}

/// The table of `report`: income, expense, the operating result, the owner
/// drawings (by category when `drawings_detail`, else their total) and the
/// result after drawings. `t` gives the package texts.
pub(crate) fn table(
    report: &Report,
    drawings_detail: bool,
    t: &dyn Fn(&str) -> String,
) -> Vec<RowView> {
    let mut rows = vec![RowView::heading(&t("budget.report.income"))];
    rows.extend(report.income.iter().map(RowView::of));
    let (budget, actual) = report.income_total();
    rows.push(RowView::figures(
        Look::Total,
        &t("budget.report.income_total"),
        BudgetKind::Income,
        budget,
        actual,
    ));
    rows.push(RowView::heading(&t("budget.report.expense")));
    rows.extend(report.expense.iter().map(RowView::of));
    let (budget, actual) = report.expense_total();
    rows.push(RowView::figures(
        Look::Total,
        &t("budget.report.expense_total"),
        BudgetKind::Expense,
        budget,
        actual,
    ));
    let (budget, actual) = report.operating();
    rows.push(RowView::figures(
        Look::Total,
        &t("budget.report.operating"),
        BudgetKind::Income,
        budget,
        actual,
    ));
    rows.push(RowView::heading(&t("budget.report.drawings")));
    if drawings_detail {
        rows.extend(report.drawings.iter().map(RowView::of));
    }
    let (budget, actual) = report.drawings_total();
    rows.push(RowView::figures(
        Look::Total,
        &t("budget.report.drawings_total"),
        BudgetKind::OwnerDrawings,
        budget,
        actual,
    ));
    let (budget, actual) = report.after_drawings();
    rows.push(RowView::figures(
        Look::Total,
        &t("budget.report.after_drawings"),
        BudgetKind::Income,
        budget,
        actual,
    ));
    rows
}

/// One line of the owner drawings, for those who may see it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrawingView {
    month: String,
    description: String,
    payee: String,
    amount: String,
    self_approved: bool,
    link: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-invoicing/budget_report.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    head: Vec<String>,
    rows: Vec<RowView>,
    /// The legal entities to filter by, `(id or "", label)`.
    entities: Vec<(String, String)>,
    /// The selected entity, `""` for all.
    entity: String,
    /// The month shown, `2027-03`.
    month: String,
    ytd: bool,
    /// The entities with no approved budget for the year, by code.
    without_budget: String,
    /// The day the mapping was last changed.
    mapping_changed: Option<String>,
    drawings_detail: bool,
    drawings: Vec<DrawingView>,
    /// The orders the sort select offers, and the one chosen (`""`: the
    /// report's own).
    sorts: Vec<(String, String)>,
    sort: String,
    /// The link of the CSV export, with the page's filters.
    export_url: String,
}

/// `GET /budget-report`: the budget, the actual and the variance, or the
/// table as a CSV file (`?export=csv`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<Response> {
    let t = |key: &str| texts::t(&lang, &format!("wl_invoicing.{key}"));
    if !user.has_permission_anywhere(permissions::BUDGETS_VIEW) {
        return Err(Error::Forbidden);
    }
    let scope = user.scopes_with::<LegalEntity>(permissions::BUDGETS_VIEW);
    let all: Vec<LegalEntity> = LegalEntity::query()
        .order_by(LegalEntity::CODE)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|e| scope.contains(&e.id))
        .collect();
    let requested = filter.entity_id();
    if requested.is_some_and(|id| !all.iter().any(|e| e.id == id)) {
        return Err(Error::Forbidden);
    }
    let chosen: Vec<&LegalEntity> = all
        .iter()
        .filter(|e| requested.is_none_or(|id| e.id == id))
        .collect();
    let ids: Vec<i64> = chosen.iter().map(|e| e.id).collect();
    let drawings_scope = user.scopes_with::<LegalEntity>(permissions::DRAWINGS_APPROVE);
    let drawings_detail = !ids.is_empty() && ids.iter().all(|id| drawings_scope.contains(id));
    let month = filter.month(today());
    let report = budget_report::build(
        &state.db,
        &config_of(&state),
        &ReportQuery {
            entities: ids,
            month,
            ytd: filter.ytd(),
            drawings_lines: drawings_detail,
            uncategorised: t("budget.report.uncategorised"),
        },
    )
    .await?;
    let sort = filter.sort();
    let report = sorted(report, sort.as_ref());
    let rows = table(&report, drawings_detail, &t);
    let directory = wl_kernel::parties::directory(&state);
    let mut drawings = Vec::with_capacity(report.drawings_items.len());
    for item in &report.drawings_items {
        let payee = match item.party_id {
            Some(id) => directory
                .find(&state.db, wl_kernel::PartyId(id))
                .await?
                .map(|p| p.display_name)
                .unwrap_or_default(),
            None => String::new(),
        };
        drawings.push(DrawingView {
            month: item.month.format("%Y-%m").to_string(),
            description: item.label.clone(),
            payee,
            amount: plain(item.amount),
            self_approved: item.self_approved,
            link: source_link(&item.source),
        });
    }
    let mut entities = vec![(String::new(), t("budget.report.all_entities"))];
    entities.extend(
        all.iter()
            .map(|e| (e.id.to_string(), format!("{} - {}", e.code, e.name_en))),
    );
    let without_budget = report
        .without_budget
        .iter()
        .filter_map(|id| all.iter().find(|e| e.id == *id))
        .map(|e| e.code.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let head = [
        "budget.report.category",
        "budget.report.budget",
        "budget.report.actual",
        "budget.report.variance",
        "budget.report.percent",
        "budget.report.flag",
    ]
    .iter()
    .map(|key| t(key))
    .collect::<Vec<String>>();
    if report_table::wants_csv(filter.export.as_deref()) {
        let flag = t("budget.report.unfavourable");
        let lines: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                vec![
                    row.name.clone(),
                    row.budget.clone(),
                    row.actual.clone(),
                    row.variance.clone(),
                    row.percent.clone(),
                    if row.unfavourable { flag.clone() } else { String::new() },
                ]
            })
            .collect();
        return Ok(report_table::csv("budget-vs-actual", &head, &lines).into_response());
    }
    let sort_columns: Vec<(&str, String)> = SORT_KEYS
        .iter()
        .zip([
            "budget.report.category",
            "budget.report.budget",
            "budget.report.actual",
            "budget.report.variance",
            "budget.report.percent",
        ])
        .map(|(key, label)| (*key, t(label)))
        .collect();
    let sorts = report_table::sort_options(
        &sort_columns,
        &t("report.own_order"),
        &t("report.up"),
        &t("report.down"),
    );
    let entity_param = requested.map(|e| e.to_string()).unwrap_or_default();
    let month_param = month.format("%Y-%m").to_string();
    let sort_param = sort.as_ref().map(Sort::param).unwrap_or_default();
    let export_url = report_table::export_link(
        "/budget-report",
        &[
            ("entity", entity_param.clone()),
            ("month", month_param.clone()),
            ("ytd", if filter.ytd() { "1".to_owned() } else { String::new() }),
            ("sort", sort_param.clone()),
        ],
    );
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        head,
        rows,
        entities,
        entity: entity_param,
        month: month_param,
        ytd: filter.ytd(),
        without_budget,
        mapping_changed: report
            .mapping_changed
            .map(|d| d.format("%Y-%m-%d").to_string()),
        drawings_detail,
        drawings,
        sorts,
        sort: sort_param,
        export_url,
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texts::english;

    fn english_text(key: &str) -> String {
        english(&format!("wl_invoicing.{key}")).to_owned()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn variances_and_percentages_are_written_with_their_sign() {
        assert_eq!(signed_text(50_000), "+500.00");
        assert_eq!(signed_text(-1_200), "-12.00");
        assert_eq!(signed_text(0), "0.00");
        assert_eq!(percent_text(Some(500)), "5.00%");
        assert_eq!(percent_text(Some(-2_550)), "-25.50%");
        assert_eq!(percent_text(None), "\u{2014}");
    }

    #[test]
    fn the_filter_reads_the_entity_the_month_and_the_year_to_date() {
        let filter = Filter {
            entity: Some("3".into()),
            month: Some("2027-03".into()),
            ytd: Some("1".into()),
            ..Filter::default()
        };
        assert_eq!(filter.entity_id(), Some(3));
        assert_eq!(filter.month(day(2027, 8, 9)), day(2027, 3, 1));
        assert!(filter.ytd());
        let empty = Filter::default();
        assert_eq!(empty.entity_id(), None);
        assert_eq!(empty.month(day(2027, 8, 9)), day(2027, 8, 1));
        assert!(!empty.ytd());
        let off = Filter {
            ytd: Some("0".into()),
            entity: Some(String::new()),
            month: Some("March".into()),
            ..Filter::default()
        };
        assert!(!off.ytd());
        assert_eq!(off.entity_id(), None);
        assert_eq!(off.month(day(2027, 8, 9)), day(2027, 8, 1));
    }

    #[test]
    fn the_categories_sort_inside_their_section() {
        let row = |budget, actual, name: &str| Row {
            category_id: Some(1),
            name: name.to_owned(),
            kind: BudgetKind::Expense,
            budget,
            actual,
        };
        let report = Report {
            month: day(2027, 3, 1),
            ytd: false,
            entities: vec![1],
            without_budget: Vec::new(),
            income: Vec::new(),
            expense: vec![
                row(1_000_000, 1_050_000, "Marketing"),
                row(500_000, 900_000, "Rent"),
                row(0, 10_000, "Fuel"),
            ],
            drawings: Vec::new(),
            drawings_items: Vec::new(),
            mapping_changed: None,
        };
        let names = |report: &Report| -> Vec<String> {
            report.expense.iter().map(|r| r.name.clone()).collect()
        };
        let by = |key: &str| sorted(report.clone(), Sort::parse(Some(key), &SORT_KEYS).as_ref());
        let by_variance = by("-variance");
        assert_eq!(names(&by_variance), ["Rent", "Marketing", "Fuel"]);
        let by_name = by("category");
        assert_eq!(names(&by_name), ["Fuel", "Marketing", "Rent"]);
        // No budget, no percentage: first when ascending.
        let by_percent = by("percent");
        assert_eq!(names(&by_percent), ["Fuel", "Marketing", "Rent"]);
        assert_eq!(names(&sorted(report.clone(), None)), names(&report));
        // The table keeps its headings and totals around the sorted lines.
        let lines = table(&by_variance, false, &english_text);
        let rent = lines.iter().position(|r| r.name == "Rent").unwrap();
        let marketing = lines.iter().position(|r| r.name == "Marketing").unwrap();
        assert!(rent < marketing);
    }

    #[test]
    fn the_table_shows_the_drawings_total_only_without_the_detail() {
        let row = |kind, budget, actual, name: &str| Row {
            category_id: Some(1),
            name: name.to_owned(),
            kind,
            budget,
            actual,
        };
        let report = Report {
            month: day(2027, 3, 1),
            ytd: false,
            entities: vec![1],
            without_budget: Vec::new(),
            income: Vec::new(),
            expense: vec![row(BudgetKind::Expense, 1_000_000, 1_050_000, "Marketing")],
            drawings: vec![row(
                BudgetKind::OwnerDrawings,
                250_000,
                300_000,
                "Partner A",
            )],
            drawings_items: Vec::new(),
            mapping_changed: None,
        };
        let without = table(&report, false, &english_text);
        // The section heading is also called "Owner drawings", so the category has its own name.
        assert!(!without.iter().any(|r| r.name == "Partner A"));
        let total = without
            .iter()
            .find(|r| r.name == "Total owner drawings")
            .expect("the drawings total");
        assert_eq!(total.actual, "3,000.00");
        let with = table(&report, true, &english_text);
        assert!(with.iter().any(|r| r.name == "Partner A"));
        let marketing = with.iter().find(|r| r.name == "Marketing").unwrap();
        assert_eq!(
            (marketing.variance.as_str(), marketing.percent.as_str()),
            ("+500.00", "5.00%")
        );
        assert!(marketing.unfavourable);
        // The result after drawings follows the operating result.
        let names: Vec<&str> = with.iter().map(|r| r.name.as_str()).collect();
        let operating = names.iter().position(|n| *n == "Operating result").unwrap();
        let after = names
            .iter()
            .position(|n| *n == "Result after drawings")
            .unwrap();
        assert!(operating < after);
    }
}
