//! The company budget (wl-invoicing §3 *Company budget*, *Company budget
//! line*, §4, §5.15 step 1, `FIN.BUD.01`): the budget of one legal entity for
//! one fiscal year, as an amount per budget category
//! ([`crate::budget_category`]) and month, in versions. This file holds the
//! models, the fiscal year arithmetic and the reads; the machine is in
//! [`crate::budget_machine`], creating a version and entering lines in
//! [`crate::budget_lines`], and the reminder of `FIN.RUL.26` in
//! [`crate::budget_reminder`].
//!
//! A **fiscal year** is 12 months from the start month in the settings and is
//! named by the calendar year of its first month ([`fiscal_months`]); a
//! version keeps the start month it was made with, so a change of the setting
//! applies only from the next fiscal year that has no version yet. Amounts
//! are AED fils without VAT. A month is stored as its first day.

use renox::chrono::{Datelike, Days, Months, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::budgets::CompanyBudgetId;
use wl_kernel::{LegalEntity, Subject};

/// Where a budget version stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BudgetStatus {
    /// Being prepared; its lines can be changed.
    #[default]
    Draft,
    /// Waiting for the approval; not editable.
    Submitted,
    /// The budget in force for its entity and fiscal year.
    Approved,
    /// Replaced by a newer approved version; kept for comparison.
    Superseded,
    /// A draft given up; used by nothing.
    Discarded,
}

/// How a version started, chosen once at `create`.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StartChoice {
    /// With no lines.
    #[default]
    Empty,
    /// As a copy of last year's approved budget, moved a year on.
    LastBudget,
    /// From last year's actual per category and month.
    LastActual,
    /// A revision: a copy of the approved version of the same year.
    Revision,
}

/// One version of the budget of one legal entity for one fiscal year.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_company_budgets",
    state_machine,
    unique(legal_entity_id, fiscal_year, version),
    index(status)
)]
pub struct CompanyBudget {
    /// The primary key.
    pub id: i64,
    /// The legal entity it is for.
    pub legal_entity_id: i64,
    /// The fiscal year, named by the calendar year of its first month.
    pub fiscal_year: i64,
    /// The month the year starts in, 1 to 12.
    pub start_month: i64,
    /// The version, from 1 per entity and year.
    pub version: i64,
    /// A name for the version.
    pub name: String,
    /// How the version started.
    pub start_choice: StartChoice,
    /// The user who made it.
    pub prepared_by: Option<i64>,
    /// The user who submitted it (the requester of the approval).
    pub submitted_by: Option<i64>,
    /// When it was submitted.
    pub submitted_at: Option<DateTime>,
    /// The user who gave the final approval.
    pub approved_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// The approval request (`PLT.APR.01`) of the latest submit.
    pub approval_request_id: Option<i64>,
    /// The earlier approved version it replaced.
    pub supersedes_id: Option<i64>,
    /// Why a revision was made.
    pub revision_reason: Option<String>,
    /// Why it was returned to `draft`.
    pub return_reason: Option<String>,
    /// Why the draft was given up.
    pub discard_reason: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: BudgetStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The budgeted amount for one category and month.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_company_budget_lines",
    unique(budget_id, category_id, month),
    index(category_id)
)]
pub struct CompanyBudgetLine {
    /// The primary key.
    pub id: i64,
    /// The budget version.
    pub budget_id: i64,
    /// The budget category.
    pub category_id: i64,
    /// The first day of the month.
    pub month: NaiveDate,
    /// The amount in AED fils, without VAT.
    pub amount: i64,
    /// A note on the line.
    pub note: Option<String>,
    /// Whether the month had ended when the version was created.
    pub locked: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CompanyBudgetLine {
    /// The lines of budget `budget_id`, by category then month.
    pub async fn of(db: &Db, budget_id: i64) -> Result<Vec<CompanyBudgetLine>> {
        CompanyBudgetLine::query()
            .where_(CompanyBudgetLine::BUDGET_ID.eq(budget_id))
            .order_by(CompanyBudgetLine::CATEGORY_ID)
            .order_by(CompanyBudgetLine::MONTH)
            .get(db)
            .await
    }
}

impl CompanyBudget {
    /// The budget's id in the kernel contract.
    pub fn budget_id(&self) -> CompanyBudgetId {
        CompanyBudgetId(self.id)
    }

    /// The budget as a [`Subject`]: `invoicing.company_budget:3`.
    pub fn subject(&self) -> Subject {
        self.budget_id().subject()
    }

    /// The start month, 1 to 12.
    pub fn start_month_u32(&self) -> u32 {
        u32::try_from(self.start_month)
            .ok()
            .filter(|m| (1..=12).contains(m))
            .unwrap_or(1)
    }

    /// The first days of the 12 months of its fiscal year.
    pub fn months(&self) -> Vec<NaiveDate> {
        fiscal_months(self.start_month_u32(), self.fiscal_year)
    }

    /// Whether it is a revision of an approved version.
    pub fn is_revision(&self) -> bool {
        self.start_choice == StartChoice::Revision
    }

    /// Whether its lines can be changed.
    pub fn is_editable(&self) -> bool {
        self.status == BudgetStatus::Draft
    }

    /// The approved version of `entity` for `fiscal_year`, if any.
    pub async fn approved_for(
        db: &Db,
        entity: i64,
        fiscal_year: i64,
    ) -> Result<Option<CompanyBudget>> {
        CompanyBudget::query()
            .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity))
            .where_(CompanyBudget::FISCAL_YEAR.eq(fiscal_year))
            .where_(CompanyBudget::STATUS.eq(BudgetStatus::Approved))
            .first(db)
            .await
    }

    /// The approved version of `entity` whose fiscal year holds the month of
    /// `date`, if any.
    pub async fn approved_on(
        db: &Db,
        entity: i64,
        date: NaiveDate,
    ) -> Result<Option<CompanyBudget>> {
        let month = month_start(date);
        let years = [i64::from(date.year()) - 1, i64::from(date.year())];
        let approved = CompanyBudget::query()
            .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity))
            .where_(CompanyBudget::FISCAL_YEAR.is_in(years))
            .where_(CompanyBudget::STATUS.eq(BudgetStatus::Approved))
            .get(db)
            .await?;
        Ok(approved
            .into_iter()
            .find(|budget| budget.months().contains(&month)))
    }

    /// The version being prepared or waiting for its approval, if any.
    pub async fn open_for(
        db: impl renox::db::Executor<'_>,
        entity: i64,
        fiscal_year: i64,
    ) -> Result<Option<CompanyBudget>> {
        CompanyBudget::query()
            .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity))
            .where_(CompanyBudget::FISCAL_YEAR.eq(fiscal_year))
            .where_(CompanyBudget::STATUS.is_in([BudgetStatus::Draft, BudgetStatus::Submitted]))
            .first(db)
            .await
    }

    /// Every version of `entity` for `fiscal_year`, oldest first.
    pub async fn versions_of(
        db: impl renox::db::Executor<'_>,
        entity: i64,
        fiscal_year: i64,
    ) -> Result<Vec<CompanyBudget>> {
        CompanyBudget::query()
            .where_(CompanyBudget::LEGAL_ENTITY_ID.eq(entity))
            .where_(CompanyBudget::FISCAL_YEAR.eq(fiscal_year))
            .order_by(CompanyBudget::VERSION)
            .get(db)
            .await
    }

    /// The start month a version of `entity` for `fiscal_year` gets: the
    /// one the year's versions have, else the setting.
    pub async fn start_month_for(
        db: impl renox::db::Executor<'_>,
        entity: i64,
        fiscal_year: i64,
        setting: u32,
    ) -> Result<u32> {
        let latest = CompanyBudget::versions_of(db, entity, fiscal_year)
            .await?
            .pop();
        Ok(latest.map_or(setting, |b| b.start_month_u32()))
    }
}

/// The first day of the month of `date`.
pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The last day of the month of `date`.
pub fn month_end(date: NaiveDate) -> NaiveDate {
    month_start(date)
        .checked_add_months(Months::new(1))
        .and_then(|next| next.checked_sub_days(Days::new(1)))
        .unwrap_or(date)
}

/// The first day of fiscal year `fiscal_year` when it starts in
/// `start_month`.
pub fn year_start(fiscal_year: i64, start_month: u32) -> NaiveDate {
    let year = i32::try_from(fiscal_year).unwrap_or(2000);
    NaiveDate::from_ymd_opt(year, start_month.clamp(1, 12), 1).unwrap_or(NaiveDate::MIN)
}

/// The first days of the 12 months of fiscal year `fiscal_year`.
pub fn fiscal_months(start_month: u32, fiscal_year: i64) -> Vec<NaiveDate> {
    let start = year_start(fiscal_year, start_month);
    (0..12)
        .filter_map(|i| start.checked_add_months(Months::new(i)))
        .collect()
}

/// The fiscal year `date` falls in when years start in `start_month`.
pub fn fiscal_year_of(start_month: u32, date: NaiveDate) -> i64 {
    let year = i64::from(date.year());
    if date.month() >= start_month.clamp(1, 12) {
        year
    } else {
        year - 1
    }
}

/// Reads an AED amount like `20000`, `20,000.50` or `0` into fils; `None`
/// for a negative number, more than two decimals or text.
pub fn parse_budget_amount(text: &str) -> Option<i64> {
    let cleaned = text.trim().replace(',', "");
    let zero = !cleaned.is_empty() && cleaned.chars().all(|c| c == '0' || c == '.');
    if zero && cleaned.matches('.').count() <= 1 && cleaned != "." {
        return Some(0);
    }
    crate::credit_note_pages::parse_amount(&cleaned)
}

/// The legal entity `id`, when it is active.
pub async fn active_entity(db: &Db, id: i64) -> Result<Option<LegalEntity>> {
    Ok(LegalEntity::find(db, id)
        .await?
        .filter(LegalEntity::is_active))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn statuses_and_choices_are_stored_in_snake_case() {
        let statuses: Vec<_> = BudgetStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            statuses,
            ["draft", "submitted", "approved", "superseded", "discarded"]
        );
        let choices: Vec<_> = StartChoice::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(choices, ["empty", "last_budget", "last_actual", "revision"]);
    }

    #[test]
    fn a_fiscal_year_runs_twelve_months_from_the_start_month() {
        let january = fiscal_months(1, 2027);
        assert_eq!(
            (january[0], january[11]),
            (day(2027, 1, 1), day(2027, 12, 1))
        );
        let july = fiscal_months(7, 2027);
        assert_eq!((july[0], july[11]), (day(2027, 7, 1), day(2028, 6, 1)));
        assert_eq!(july.len(), 12);
    }

    #[test]
    fn a_date_belongs_to_the_fiscal_year_that_started_before_it() {
        assert_eq!(fiscal_year_of(1, day(2027, 12, 31)), 2027);
        assert_eq!(fiscal_year_of(7, day(2027, 6, 30)), 2026);
        assert_eq!(fiscal_year_of(7, day(2027, 7, 1)), 2027);
        assert_eq!(fiscal_year_of(7, day(2028, 1, 15)), 2027);
    }

    #[test]
    fn months_start_and_end_on_their_days() {
        assert_eq!(month_start(day(2027, 2, 17)), day(2027, 2, 1));
        assert_eq!(month_end(day(2027, 2, 17)), day(2027, 2, 28));
        assert_eq!(month_end(day(2028, 2, 1)), day(2028, 2, 29));
        assert_eq!(month_end(day(2027, 12, 5)), day(2027, 12, 31));
    }

    #[test]
    fn amounts_are_read_in_dirhams() {
        assert_eq!(parse_budget_amount("20000"), Some(2_000_000));
        assert_eq!(parse_budget_amount("20,000.50"), Some(2_000_050));
        assert_eq!(parse_budget_amount("0"), Some(0));
        assert_eq!(parse_budget_amount("0.00"), Some(0));
        assert_eq!(parse_budget_amount("-5"), None);
        assert_eq!(parse_budget_amount("1.234"), None);
        assert_eq!(parse_budget_amount("abc"), None);
        assert_eq!(parse_budget_amount(""), None);
        assert_eq!(parse_budget_amount("."), None);
    }

    #[test]
    fn a_version_knows_its_months_and_whether_it_is_a_revision() {
        let budget = CompanyBudget {
            fiscal_year: 2027,
            start_month: 4,
            start_choice: StartChoice::Revision,
            ..Default::default()
        };
        assert_eq!(budget.months()[0], day(2027, 4, 1));
        assert!(budget.is_revision());
        assert!(budget.is_editable());
        let broken = CompanyBudget {
            start_month: 40,
            ..Default::default()
        };
        assert_eq!(broken.start_month_u32(), 1);
    }
}
