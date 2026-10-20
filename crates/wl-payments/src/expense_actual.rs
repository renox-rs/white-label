//! The approved expense claim lines in the actual of the company budget
//! (wl-invoicing §5.15 step 2, `FIN.BUD.01`): "approved expense claim lines by
//! expense date (expense category, wl-payments)".
//!
//! wl-invoicing computes budget vs actual and reads the amounts other modules
//! own through the kernel [`ActualCostSource`]: each line of an *approved*
//! claim is one amount of the claim's legal entity, dated the day of the
//! expense, keyed by its expense category code (`expense_category`), which the
//! budget mapping turns into a budget category (a category of kind
//! `owner_drawings` puts it on the drawings line, never in expense). The
//! amount is without recoverable VAT: the VAT of a line whose category
//! recovers it is taken off; a category whose VAT is blocked (the *Owner
//! drawings* one among them) counts the line with its VAT.
//!
//! Decided where the spec is silent: a claim counts from its approval (the
//! claim lifecycle of this build ends there; reimbursement is not built), a
//! submitted, returned or rejected claim does not; a line whose category no
//! longer exists keeps its VAT, as nothing says it is recovered.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::budgets::{ActualCost, ActualCostSource, BudgetFuture, register_actual_costs};

use crate::expense_category::{ExpenseCategory, VatTreatment};
use crate::expense_claim::{ClaimStatus, ExpenseClaim, ExpenseLine};

/// The name the source is registered under.
pub const SOURCE: &str = "wl-payments.expense_claims";

/// The key kind of its amounts.
pub const KEY_KIND: &str = "expense_category";

/// The approved expense claim lines.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExpenseClaimCosts;

/// Registers [`ExpenseClaimCosts`] with the kernel.
pub(crate) fn register() {
    register_actual_costs(SOURCE, ExpenseClaimCosts);
}

/// What `line` counts in the actual: its amount, less its VAT when the
/// category recovers it.
pub fn counted(line: &ExpenseLine, treatment: Option<VatTreatment>) -> i64 {
    match treatment {
        Some(VatTreatment::Recoverable) => line.amount - line.vat_amount,
        Some(VatTreatment::Blocked | VatTreatment::NoVat) | None => line.amount,
    }
}

impl ActualCostSource for ExpenseClaimCosts {
    fn costs<'a>(
        &'a self,
        db: &'a Db,
        entities: &'a [i64],
        from: NaiveDate,
        to: NaiveDate,
    ) -> BudgetFuture<'a, Vec<ActualCost>> {
        Box::pin(async move {
            if entities.is_empty() || to < from {
                return Ok(Vec::new());
            }
            let claims: BTreeMap<i64, ExpenseClaim> = ExpenseClaim::query()
                .where_(ExpenseClaim::STATUS.eq(ClaimStatus::Approved))
                .where_(ExpenseClaim::LEGAL_ENTITY_ID.is_in(entities.to_vec()))
                .get(db)
                .await?
                .into_iter()
                .map(|c| (c.id, c))
                .collect();
            if claims.is_empty() {
                return Ok(Vec::new());
            }
            let treatments: BTreeMap<String, VatTreatment> = ExpenseCategory::query()
                .get(db)
                .await?
                .into_iter()
                .map(|c| (c.code, c.vat_treatment))
                .collect();
            let lines = ExpenseLine::query()
                .where_(ExpenseLine::CLAIM_ID.is_in(claims.keys().copied().collect::<Vec<_>>()))
                .where_(ExpenseLine::EXPENSE_DATE.gte(from))
                .where_(ExpenseLine::EXPENSE_DATE.lte(to))
                .order_by(ExpenseLine::CLAIM_ID)
                .order_by(ExpenseLine::POSITION)
                .get(db)
                .await?;
            let mut costs = Vec::with_capacity(lines.len());
            for line in lines {
                let Some(claim) = claims.get(&line.claim_id) else {
                    continue;
                };
                let amount = counted(&line, treatments.get(&line.category).copied());
                if amount == 0 {
                    continue;
                }
                let label = if line.description.trim().is_empty() {
                    line.merchant.clone()
                } else {
                    format!("{}: {}", line.merchant, line.description)
                };
                costs.push(ActualCost {
                    legal_entity_id: claim.legal_entity_id,
                    date: line.expense_date,
                    key_kind: KEY_KIND.to_owned(),
                    key: line.category.clone(),
                    amount,
                    label,
                    source: claim.subject(),
                });
            }
            Ok(costs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(amount: i64, vat_amount: i64) -> ExpenseLine {
        ExpenseLine {
            amount,
            vat_amount,
            ..Default::default()
        }
    }

    #[test]
    fn recoverable_vat_is_taken_off_and_blocked_vat_stays() {
        let receipt = line(10_500, 500);
        assert_eq!(counted(&receipt, Some(VatTreatment::Recoverable)), 10_000);
        assert_eq!(counted(&receipt, Some(VatTreatment::Blocked)), 10_500);
        assert_eq!(counted(&line(3_000, 0), Some(VatTreatment::NoVat)), 3_000);
        assert_eq!(counted(&receipt, None), 10_500);
    }
}
