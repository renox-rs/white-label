//! The company budget contract (ARCHITECTURE §5, wl-invoicing §3 *Company
//! budget*, §5.15, §6 `CompanyBudgets`, `FIN.BUD.01`): what other modules and
//! the verticals may ask of the company budget without depending on the
//! `wl-invoicing` crate.
//!
//! `wl-invoicing` provides [`CompanyBudgets`] with [`provide_company_budgets`]
//! and emits [`CompanyBudgetApproved`] when a budget version is approved.
//! [`company_budgets`] falls back to [`NoCompanyBudgets`] when `invoicing` is
//! off: there is no approved budget and no budget category is owner
//! drawings.
//!
//! A **fiscal year** is named by the calendar year of its first month: with
//! the start month in the settings at July, fiscal year 2027 runs from
//! 2027-07-01 to 2028-06-30. Money is `i64` in fils of AED, without VAT.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// The subject type of a company budget version, for [`Subject`]
/// references and the approval requests about it.
pub const COMPANY_BUDGET_SUBJECT_TYPE: &str = "invoicing.company_budget";

/// The id of a company budget version.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct CompanyBudgetId(pub i64);

impl CompanyBudgetId {
    /// The budget as a [`Subject`]: `invoicing.company_budget:3`.
    pub fn subject(self) -> Subject {
        Subject::new(COMPANY_BUDGET_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for CompanyBudgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for CompanyBudgetId {
    fn from(id: i64) -> Self {
        CompanyBudgetId(id)
    }
}

/// A budget version was approved and is now the budget in force for its
/// entity and fiscal year (`approve`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyBudgetApproved {
    /// The approved version.
    pub budget: CompanyBudgetId,
    /// The legal entity it is for.
    pub legal_entity_id: i64,
    /// The fiscal year, named by the calendar year of its first month.
    pub fiscal_year: i32,
    /// The version number, from 1.
    pub version: i64,
    /// The earlier approved version it replaced, if any.
    pub superseded: Option<CompanyBudgetId>,
}

impl Event for CompanyBudgetApproved {}

/// An actual reached its budget (`FIN.RUL.24`): the month's actual expense or
/// drawings of a budget category reached the percentage of the approved
/// budget line for the entity, category and month. Sent once per entity,
/// category and month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyBudgetVarianceReached {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The code of the budget category.
    pub category: String,
    /// The first day of the month.
    pub month: NaiveDate,
    /// The approved budget line, in AED fils (0 when the category has no
    /// line).
    pub budget: i64,
    /// The actual of the month so far, in AED fils.
    pub actual: i64,
    /// The share of the budget used, in whole percent; `None` against a
    /// budget of 0.
    pub percent_used: Option<i64>,
}

impl Event for CompanyBudgetVarianceReached {}

/// The future the [`CompanyBudgets`] reads return.
pub type BudgetFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What other modules ask of the company budget. `wl-invoicing` provides it;
/// without that module, [`company_budgets`] gives [`NoCompanyBudgets`].
pub trait CompanyBudgets: Send + Sync + 'static {
    /// The budgeted amount, in fils of AED without VAT, of the approved
    /// budget of `legal_entity_id` for the budget category `category` (its
    /// code) in the month that contains `month`. `None` when the entity has
    /// no approved budget for that fiscal year; `Some(0)` when it has one
    /// and the category has no line (a category with no line counts as a
    /// budget of 0). The default has no budget.
    fn approved_amount<'a>(
        &'a self,
        _db: &'a Db,
        _legal_entity_id: i64,
        _category: &'a str,
        _month: NaiveDate,
    ) -> BudgetFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }

    /// The actual of the budget category with this code for `legal_entity_id`
    /// in the month that contains `month`, in AED fils without recoverable
    /// VAT (`FIN.BUD.01`, wl-invoicing §5.15 step 2): issued invoices less
    /// credit notes for income, approved supplier bills less debit notes for
    /// expense and owner drawings. The default has none (0).
    fn actual_amount<'a>(
        &'a self,
        _db: &'a Db,
        _legal_entity_id: i64,
        _category: &'a str,
        _month: NaiveDate,
    ) -> BudgetFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    /// Whether the budget category with this code is of kind
    /// `owner_drawings`. The default says no category is.
    fn is_owner_drawings<'a>(&'a self, _db: &'a Db, _category: &'a str) -> BudgetFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The company budgets when the `invoicing` module is off: no approved
/// budget, no owner drawings category.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCompanyBudgets;

impl CompanyBudgets for NoCompanyBudgets {}

/// Makes `budgets` the app's [`CompanyBudgets`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_company_budgets(app: &mut Registry, budgets: impl CompanyBudgets) {
    let budgets: Arc<dyn CompanyBudgets> = Arc::new(budgets);
    app.provide(budgets);
}

/// The app's [`CompanyBudgets`], or [`NoCompanyBudgets`] when none is
/// provided.
pub fn company_budgets(state: &AppState) -> Arc<dyn CompanyBudgets> {
    match state.provided::<Arc<dyn CompanyBudgets>>() {
        Some(budgets) => Arc::clone(&budgets),
        None => Arc::new(NoCompanyBudgets),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variance_event_has_no_percentage_against_a_budget_of_zero() {
        let event = CompanyBudgetVarianceReached {
            legal_entity_id: 1,
            category: "marketing".into(),
            month: NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(),
            budget: 0,
            actual: 80_000,
            percent_used: None,
        };
        let json = renox::serde_json::to_value(&event).unwrap();
        assert_eq!(json["percent_used"], renox::serde_json::Value::Null);
        assert_eq!(json["month"], "2027-03-01");
    }

    #[test]
    fn a_budget_id_is_a_subject_and_a_plain_number() {
        let id = CompanyBudgetId::from(3);
        assert_eq!(id.to_string(), "3");
        assert_eq!(id.subject(), Subject::new("invoicing.company_budget", 3));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "3");
    }
}
