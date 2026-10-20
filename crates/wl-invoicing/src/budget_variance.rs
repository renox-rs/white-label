//! The warning when an actual reaches its budget (wl-invoicing §6
//! `FIN.RUL.24`, `FIN.BUD.01`).
//!
//! When an approved supplier bill posts an actual to a budget category of
//! kind `expense` or `owner_drawings` ([`after_bill_approved`], run from
//! `SupplierBillApproved`), [`check`] compares the month's actual with the
//! approved budget line of the entity, category and month. It warns when the
//! actual reaches P% of the line ([`crate::InvoicingConfig::budget_variance_percent`],
//! default 100); a category with no line counts as a budget of 0, so the first
//! expense posted in the month warns. The category's owner role and the
//! finance manager ([`crate::InvoicingConfig::budget_variance_role`]) are
//! told, and `CompanyBudgetVarianceReached` is emitted, once per entity,
//! category and month: later postings do not warn again, and an actual that
//! goes back below the share does not clear the warning ([`BudgetWarning`]).
//! The rule does not run for an entity and year with no approved budget, nor
//! when nobody holds the roles to tell.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::budgets::CompanyBudgetVarianceReached;
use wl_kernel::documents::today;
use wl_kernel::invoicing::SupplierBillApproved;
use wl_kernel::notify::{self, NotifyRequest};
use wl_kernel::{LegalEntity, Subject};

use crate::allocate::amount_text;
use crate::budget::month_start;
use crate::budget_actual::{self, Mapped};
use crate::budget_category::{BudgetCategory, BudgetKind, MappingKind};
use crate::budget_contract::approved_amount;
use crate::budget_notices::{VARIANCE, holders_of};
use crate::config_of;
use crate::supplier_bill::SupplierBill;
use crate::vat::div_round_half_up;

/// The rule, as the specification names it.
pub const RULE: &str = "FIN.RUL.24";

/// A warning that went out.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_budget_warnings",
    unique(legal_entity_id, category_id, month)
)]
pub struct BudgetWarning {
    /// The primary key.
    pub id: i64,
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The budget category.
    pub category_id: i64,
    /// The first day of the month.
    pub month: NaiveDate,
    /// The approved line when it went out, in AED fils.
    pub budget: i64,
    /// The actual when it went out, in AED fils.
    pub actual: i64,
    /// The day it went out.
    pub sent_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Whether `actual` reaches `percent`% of `budget`: it has to be above 0, so
/// a month with nothing posted never warns against a budget of 0.
pub fn reached(actual: i64, budget: i64, percent: u32) -> bool {
    actual > 0 && i128::from(actual) * 100 >= i128::from(budget) * i128::from(percent)
}

/// The share of `budget` that `actual` is, in whole percent (rounded half
/// up); `None` against a budget of 0.
pub fn percent_used(actual: i64, budget: i64) -> Option<i64> {
    (budget > 0).then(|| div_round_half_up(i128::from(actual) * 100, i128::from(budget)))
}

/// Warns, once, if the actual of `category` for `entity` in the month of
/// `month` has reached the share of its approved budget line. Returns whether
/// a warning went out.
pub async fn check(
    state: &AppState,
    entity: i64,
    category: &BudgetCategory,
    month: NaiveDate,
) -> Result<bool> {
    if category.kind == BudgetKind::Income {
        return Ok(false);
    }
    let month = month_start(month);
    let Some(budget) = approved_amount(&state.db, entity, &category.code, month).await? else {
        return Ok(false);
    };
    let actual = budget_actual::category_actual(&state.db, entity, category.id, month).await?;
    let config = config_of(state);
    if !reached(actual, budget, config.budget_variance_percent) {
        return Ok(false);
    }
    let sent = BudgetWarning::query()
        .where_(BudgetWarning::LEGAL_ENTITY_ID.eq(entity))
        .where_(BudgetWarning::CATEGORY_ID.eq(category.id))
        .where_(BudgetWarning::MONTH.eq(month))
        .exists(&state.db)
        .await?;
    if sent {
        return Ok(false);
    }
    let mut roles = vec![config.budget_variance_role.clone()];
    roles.extend(category.owner_role.clone());
    let recipients = holders_of(&state.db, &roles).await?;
    if recipients.is_empty() {
        return Ok(false);
    }
    let code = LegalEntity::find(&state.db, entity)
        .await?
        .map(|e| e.code)
        .unwrap_or_default();
    let percent = percent_used(actual, budget);
    let mut request = NotifyRequest::new(VARIANCE, Subject::new("kernel.legal_entity", entity))
        .data(json!({
            "entity": code,
            "category": category.name_en,
            "month": month.format("%Y-%m").to_string(),
            "actual": amount_text("AED", actual),
            "budget": amount_text("AED", budget),
            "percent": percent.map_or_else(|| "no budget line".to_owned(), |p| format!("{p}%")),
            "link": format!("/budget-report?entity={entity}&month={}", month.format("%Y-%m")),
        }));
    for id in recipients {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    BudgetWarning {
        legal_entity_id: entity,
        category_id: category.id,
        month,
        budget,
        actual,
        sent_on: today(),
        ..Default::default()
    }
    .insert(&state.db)
    .await?;
    state
        .emit(CompanyBudgetVarianceReached {
            legal_entity_id: entity,
            category: category.code.clone(),
            month,
            budget,
            actual,
            percent_used: percent,
        })
        .await?;
    Ok(true)
}

/// Checks the budget categories the lines of supplier bill `bill_id` post an
/// actual to. Returns how many warnings went out.
pub async fn after_bill_approved(state: &AppState, bill_id: i64) -> Result<usize> {
    let Some(bill) = SupplierBill::find(&state.db, bill_id).await? else {
        return Ok(0);
    };
    let Some(entity) = bill.legal_entity_id else {
        return Ok(0);
    };
    let mapped = Mapped::load(&state.db).await?;
    let mut categories: Vec<i64> = crate::supplier_bill::lines_of(&state.db, bill.id)
        .await?
        .iter()
        .filter_map(|line| mapped.find(MappingKind::CostCode, line.cost_code.as_deref()))
        .filter(|(_, kind)| *kind != BudgetKind::Income)
        .map(|(id, _)| id)
        .collect();
    categories.sort_unstable();
    categories.dedup();
    let mut sent = 0;
    for id in categories {
        if let Some(category) = BudgetCategory::find(&state.db, id).await? {
            sent += usize::from(check(state, entity, &category, bill.bill_date).await?);
        }
    }
    Ok(sent)
}

/// Registers the listener of `SupplierBillApproved`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SupplierBillApproved, state: AppState| async move {
        after_bill_approved(&state, event.bill.0).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_comes_at_the_share_of_the_budget() {
        assert!(!reached(999_999, 1_000_000, 100));
        assert!(reached(1_000_000, 1_000_000, 100));
        assert!(reached(1_050_000, 1_000_000, 100));
        assert!(reached(900_000, 1_000_000, 90));
        assert!(!reached(899_999, 1_000_000, 90));
    }

    #[test]
    fn a_category_without_a_line_warns_at_the_first_posting_only() {
        assert!(reached(1, 0, 100));
        assert!(!reached(0, 0, 100));
        assert!(!reached(-500, 0, 100));
    }

    #[test]
    fn the_share_used_is_a_whole_percent_of_a_budget_above_zero() {
        assert_eq!(percent_used(1_050_000, 1_000_000), Some(105));
        assert_eq!(percent_used(300_000, 250_000), Some(120));
        assert_eq!(percent_used(1, 0), None);
    }
}
