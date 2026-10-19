//! The budget vs actual report (wl-invoicing §5.15 steps 2 and 3,
//! `FIN.BUD.01`, `FIN.ACL.22`): per category, for a month or the year to
//! date, the approved budget, the actual ([`crate::budget_actual`]) and the
//! variance in AED and %.
//!
//! Income comes first, then expense, each with its *Uncategorised* row (the
//! documents whose mapping finds no category) and a total; the **operating
//! result** is income less expense. Owner drawings follow on their own line,
//! then the result after drawings; they are never part of the operating
//! result. The variance is the actual less the budget, *unfavourable* when
//! income is below or expense above the budget; against a budget of 0 the
//! percentage is `None`, never 0 or infinity. An entity with no approved
//! budget for the year counts as a budget of 0 and is listed in
//! [`Report::without_budget`]. The report adds the entities it is given up
//! (no elimination between them, §11); the caller limits them to the user's
//! scope. The lines of the drawings are in [`Report::drawings_items`] only
//! when the caller asks for them (`FIN.ACL.22`).

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::dubai_date;

use crate::budget::{CompanyBudget, CompanyBudgetLine, fiscal_year_of, month_end, month_start, year_start};
use crate::budget_actual::{self, ActualItem};
use crate::budget_category::{BudgetCategory, BudgetKind, BudgetMapping};
use crate::config::InvoicingConfig;
use crate::vat::div_round_half_up;

/// What to report on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportQuery {
    /// The legal entities, already limited to the user's scope.
    pub entities: Vec<i64>,
    /// Any day of the month reported.
    pub month: NaiveDate,
    /// The year to date up to the month, else the month alone.
    pub ytd: bool,
    /// Whether the lines of the drawings are wanted (`drawings.approve`).
    pub drawings_lines: bool,
    /// The name of the *Uncategorised* rows.
    pub uncategorised: String,
}

/// One row of the report: a category, or *Uncategorised*.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The category; `None`: *Uncategorised*.
    pub category_id: Option<i64>,
    /// Its name.
    pub name: String,
    /// Income, expense or owner drawings.
    pub kind: BudgetKind,
    /// The approved budget, in AED fils.
    pub budget: i64,
    /// The actual, in AED fils.
    pub actual: i64,
}

impl Row {
    /// The actual less the budget.
    pub fn variance(&self) -> i64 {
        self.actual - self.budget
    }

    /// The variance in basis points of the budget; `None` against a budget
    /// of 0.
    pub fn percent_bp(&self) -> Option<i64> {
        percent_bp(self.variance(), self.budget)
    }

    /// Whether the variance is unfavourable: income below, expense or
    /// drawings above the budget.
    pub fn unfavourable(&self) -> bool {
        unfavourable(self.kind, self.variance())
    }
}

/// The variance in basis points of the budget; `None` against a budget of 0.
pub fn percent_bp(variance: i64, budget: i64) -> Option<i64> {
    (budget != 0).then(|| div_round_half_up(i128::from(variance) * 10_000, i128::from(budget)))
}

/// Whether `variance` (actual less budget) is unfavourable for `kind`.
pub fn unfavourable(kind: BudgetKind, variance: i64) -> bool {
    match kind {
        BudgetKind::Income => variance < 0,
        BudgetKind::Expense | BudgetKind::OwnerDrawings => variance > 0,
    }
}

/// The report.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The first day of the month reported.
    pub month: NaiveDate,
    /// Whether it is the year to date.
    pub ytd: bool,
    /// The entities added up.
    pub entities: Vec<i64>,
    /// The entities with no approved budget for the year.
    pub without_budget: Vec<i64>,
    /// The income rows.
    pub income: Vec<Row>,
    /// The expense rows.
    pub expense: Vec<Row>,
    /// The owner drawings rows, by category.
    pub drawings: Vec<Row>,
    /// The lines of the owner drawings, when they were asked for.
    pub drawings_items: Vec<ActualItem>,
    /// The day the mapping was last changed (Asia/Dubai).
    pub mapping_changed: Option<NaiveDate>,
}

/// The budget and the actual added up.
fn sum(rows: &[Row]) -> (i64, i64) {
    rows.iter()
        .fold((0, 0), |(b, a), row| (b + row.budget, a + row.actual))
}

impl Report {
    /// The budget and actual of income.
    pub fn income_total(&self) -> (i64, i64) {
        sum(&self.income)
    }

    /// The budget and actual of expense.
    pub fn expense_total(&self) -> (i64, i64) {
        sum(&self.expense)
    }

    /// The budget and actual of the owner drawings.
    pub fn drawings_total(&self) -> (i64, i64) {
        sum(&self.drawings)
    }

    /// Income less expense: owner drawings are not part of it.
    pub fn operating(&self) -> (i64, i64) {
        let (income_budget, income_actual) = self.income_total();
        let (expense_budget, expense_actual) = self.expense_total();
        (income_budget - expense_budget, income_actual - expense_actual)
    }

    /// The operating result less the owner drawings.
    pub fn after_drawings(&self) -> (i64, i64) {
        let (budget, actual) = self.operating();
        let (drawings_budget, drawings_actual) = self.drawings_total();
        (budget - drawings_budget, actual - drawings_actual)
    }
}

/// The rows of income, expense and drawings: every category with a budget
/// or an actual, in the order of the categories, then the *Uncategorised*
/// rows of income and expense when they have an actual. `actual` is keyed by
/// category and kind (an uncategorised item has no category).
pub fn assemble(
    categories: &[BudgetCategory],
    budget: &BTreeMap<i64, i64>,
    actual: &BTreeMap<(Option<i64>, BudgetKind), i64>,
    uncategorised: &str,
) -> (Vec<Row>, Vec<Row>, Vec<Row>) {
    let (mut income, mut expense, mut drawings) = (Vec::new(), Vec::new(), Vec::new());
    for category in categories {
        let row = Row {
            category_id: Some(category.id),
            name: category.name_en.clone(),
            kind: category.kind,
            budget: budget.get(&category.id).copied().unwrap_or(0),
            actual: actual
                .get(&(Some(category.id), category.kind))
                .copied()
                .unwrap_or(0),
        };
        if row.budget == 0 && row.actual == 0 {
            continue;
        }
        match category.kind {
            BudgetKind::Income => income.push(row),
            BudgetKind::Expense => expense.push(row),
            BudgetKind::OwnerDrawings => drawings.push(row),
        }
    }
    for (kind, rows) in [
        (BudgetKind::Income, &mut income),
        (BudgetKind::Expense, &mut expense),
    ] {
        let amount = actual.get(&(None, kind)).copied().unwrap_or(0);
        if amount != 0 {
            rows.push(Row {
                category_id: None,
                name: uncategorised.to_owned(),
                kind,
                budget: 0,
                actual: amount,
            });
        }
    }
    (income, expense, drawings)
}

/// The first month of the period reported for an entity: the start of its
/// fiscal year for the year to date, else the month itself.
fn period_start(
    budget: Option<&CompanyBudget>,
    config: &InvoicingConfig,
    month: NaiveDate,
    ytd: bool,
) -> NaiveDate {
    if !ytd {
        return month_start(month);
    }
    match budget {
        Some(budget) => year_start(budget.fiscal_year, budget.start_month_u32()),
        None => {
            let start = config.budget_fiscal_start_month;
            year_start(fiscal_year_of(start, month), start)
        }
    }
}

/// Builds the report.
pub async fn build(db: &Db, config: &InvoicingConfig, query: &ReportQuery) -> Result<Report> {
    let month = month_start(query.month);
    let categories = BudgetCategory::listed(db, true).await?;
    let mut budget_sums: BTreeMap<i64, i64> = BTreeMap::new();
    let mut actual_sums: BTreeMap<(Option<i64>, BudgetKind), i64> = BTreeMap::new();
    let mut without_budget = Vec::new();
    let mut drawings_items = Vec::new();
    for entity in &query.entities {
        let budget = CompanyBudget::approved_on(db, *entity, month).await?;
        let start = period_start(budget.as_ref(), config, month, query.ytd);
        match &budget {
            Some(budget) => {
                for line in CompanyBudgetLine::of(db, budget.id).await? {
                    if line.month >= start && line.month <= month {
                        *budget_sums.entry(line.category_id).or_insert(0) += line.amount;
                    }
                }
            }
            None => without_budget.push(*entity),
        }
        for item in budget_actual::items(db, &[*entity], start, month_end(month)).await? {
            *actual_sums
                .entry((item.category_id, item.kind))
                .or_insert(0) += item.amount;
            if query.drawings_lines && item.kind == BudgetKind::OwnerDrawings {
                drawings_items.push(item);
            }
        }
    }
    let (income, expense, drawings) =
        assemble(&categories, &budget_sums, &actual_sums, &query.uncategorised);
    let mapping_changed = BudgetMapping::query()
        .order_by_desc(BudgetMapping::UPDATED_AT)
        .first(db)
        .await?
        .and_then(|m| m.updated_at)
        .map(dubai_date);
    Ok(Report {
        month,
        ytd: query.ytd,
        entities: query.entities.clone(),
        without_budget,
        income,
        expense,
        drawings,
        drawings_items,
        mapping_changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: i64, name: &str, kind: BudgetKind) -> BudgetCategory {
        BudgetCategory {
            id,
            name_en: name.to_owned(),
            kind,
            active: true,
            ..Default::default()
        }
    }

    fn row(kind: BudgetKind, budget: i64, actual: i64) -> Row {
        Row {
            category_id: Some(1),
            name: "x".into(),
            kind,
            budget,
            actual,
        }
    }

    #[test]
    fn the_variance_is_the_actual_less_the_budget_in_percent_of_the_budget() {
        let marketing = row(BudgetKind::Expense, 1_000_000, 1_050_000);
        assert_eq!(marketing.variance(), 50_000);
        assert_eq!(marketing.percent_bp(), Some(500));
        assert!(marketing.unfavourable());
        let sales = row(BudgetKind::Income, 1_000_000, 1_050_000);
        assert!(!sales.unfavourable());
        assert!(row(BudgetKind::Income, 1_000_000, 900_000).unfavourable());
        assert!(!row(BudgetKind::Expense, 1_000_000, 1_000_000).unfavourable());
    }

    #[test]
    fn a_budget_of_zero_has_no_percentage() {
        let fuel = row(BudgetKind::Expense, 0, 80_000);
        assert_eq!(fuel.variance(), 80_000);
        assert_eq!(fuel.percent_bp(), None);
        assert!(fuel.unfavourable());
        assert_eq!(percent_bp(0, 0), None);
        assert_eq!(percent_bp(-250, 1_000), Some(-2_500));
    }

    #[test]
    fn the_rows_hold_the_categories_with_figures_and_the_uncategorised_ones() {
        let categories = [
            category(1, "Sales", BudgetKind::Income),
            category(2, "Marketing", BudgetKind::Expense),
            category(3, "Fuel", BudgetKind::Expense),
            category(4, "Owner drawings", BudgetKind::OwnerDrawings),
        ];
        let budget = BTreeMap::from([(2, 1_000_000), (4, 250_000)]);
        let actual = BTreeMap::from([
            ((Some(1), BudgetKind::Income), 5_000_000),
            ((Some(2), BudgetKind::Expense), 1_050_000),
            ((Some(4), BudgetKind::OwnerDrawings), 300_000),
            ((None, BudgetKind::Income), 12_000),
            ((None, BudgetKind::Expense), 7_000),
        ]);
        let (income, expense, drawings) = assemble(&categories, &budget, &actual, "Uncategorised");
        let names = |rows: &[Row]| rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&income), ["Sales", "Uncategorised"]);
        assert_eq!(names(&expense), ["Marketing", "Uncategorised"]);
        assert_eq!(names(&drawings), ["Owner drawings"]);
        assert_eq!(expense[0].variance(), 50_000);
        assert_eq!(drawings[0].variance(), 50_000);
    }

    #[test]
    fn the_operating_result_leaves_the_drawings_out() {
        let report = Report {
            month: NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(),
            ytd: false,
            entities: vec![1],
            without_budget: Vec::new(),
            income: vec![row(BudgetKind::Income, 8_000_000, 9_000_000)],
            expense: vec![row(BudgetKind::Expense, 5_000_000, 4_000_000)],
            drawings: vec![row(BudgetKind::OwnerDrawings, 250_000, 300_000)],
            drawings_items: Vec::new(),
            mapping_changed: None,
        };
        assert_eq!(report.operating(), (3_000_000, 5_000_000));
        assert_eq!(report.drawings_total(), (250_000, 300_000));
        assert_eq!(report.after_drawings(), (2_750_000, 4_700_000));
        assert_eq!(report.income_total(), (8_000_000, 9_000_000));
        assert_eq!(report.expense_total(), (5_000_000, 4_000_000));
    }

    #[test]
    fn the_year_to_date_starts_with_the_fiscal_year() {
        let config = InvoicingConfig::default();
        let march = NaiveDate::from_ymd_opt(2027, 3, 17).unwrap();
        assert_eq!(
            period_start(None, &config, march, true),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
        assert_eq!(
            period_start(None, &config, march, false),
            NaiveDate::from_ymd_opt(2027, 3, 1).unwrap()
        );
        let april = CompanyBudget {
            fiscal_year: 2026,
            start_month: 4,
            ..Default::default()
        };
        assert_eq!(
            period_start(Some(&april), &config, march, true),
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        );
    }
}
