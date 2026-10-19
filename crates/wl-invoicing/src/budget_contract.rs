//! The company budgets other modules ask for through the kernel
//! (`wl_kernel::budgets::CompanyBudgets`, wl-invoicing §6): the approved
//! amount per entity, category and month, and whether a category is owner
//! drawings.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::budgets::{BudgetFuture, CompanyBudgets};

use crate::budget::{CompanyBudget, CompanyBudgetLine, month_start};
use crate::budget_category::{BudgetCategory, MappingKind};

/// The provider of [`CompanyBudgets`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Budgets;

/// The approved budget of `entity` for the category with code `category` in
/// the month of `month`, in AED fils: `None` without an approved budget for
/// that fiscal year, `Some(0)` for a category the budget has no line for.
pub async fn approved_amount(
    db: &Db,
    entity: i64,
    category: &str,
    month: NaiveDate,
) -> Result<Option<i64>> {
    let Some(budget) = CompanyBudget::approved_on(db, entity, month).await? else {
        return Ok(None);
    };
    let Some(category) = BudgetCategory::by_code(db, category).await? else {
        return Ok(Some(0));
    };
    let amount = CompanyBudgetLine::query()
        .where_(CompanyBudgetLine::BUDGET_ID.eq(budget.id))
        .where_(CompanyBudgetLine::CATEGORY_ID.eq(category.id))
        .where_(CompanyBudgetLine::MONTH.eq(month_start(month)))
        .sum::<i64, _>(db, "amount")
        .await?;
    Ok(Some(amount))
}

impl CompanyBudgets for Budgets {
    fn approved_amount<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
        category: &'a str,
        month: NaiveDate,
    ) -> BudgetFuture<'a, Option<i64>> {
        Box::pin(approved_amount(db, legal_entity_id, category, month))
    }

    fn actual_amount<'a>(
        &'a self,
        db: &'a Db,
        legal_entity_id: i64,
        category: &'a str,
        month: NaiveDate,
    ) -> BudgetFuture<'a, i64> {
        Box::pin(async move {
            match BudgetCategory::by_code(db, category).await? {
                Some(category) => {
                    crate::budget_actual::category_actual(db, legal_entity_id, category.id, month)
                        .await
                }
                None => Ok(0),
            }
        })
    }

    fn is_owner_drawings_code<'a>(&'a self, db: &'a Db, code: &'a str) -> BudgetFuture<'a, bool> {
        Box::pin(async move {
            Ok(
                crate::budget_category::category_of(db, MappingKind::CostCode, code)
                    .await?
                    .is_some_and(|c| c.is_drawings()),
            )
        })
    }

    fn is_owner_drawings<'a>(&'a self, db: &'a Db, category: &'a str) -> BudgetFuture<'a, bool> {
        Box::pin(async move {
            Ok(BudgetCategory::by_code(db, category)
                .await?
                .is_some_and(|c| c.is_drawings()))
        })
    }
}
