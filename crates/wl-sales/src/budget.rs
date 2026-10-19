//! The budget check of a cost entry and the `SAL.RUL.10` alerts (#138 A12,
//! A16).
//!
//! The check is made by [`crate::costs`] under the campaign lock and given
//! to the machine as its input, so the `submit` and `submit_over_budget`
//! guards agree with it. The alerts are recorded under the lock after an
//! entry entered `approved`: one row per campaign and threshold, never
//! deleted, so a later rise past the same threshold is not alerted again.

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::campaign::Campaign;
use crate::config::SalesConfig;
use crate::{costs, notices};

/// What the machine is told about the budget when an entry is submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetCheck {
    /// The campaign's approved costs, before this entry.
    pub approved: i64,
    /// The campaign's budget; empty means no check.
    pub budget: Option<i64>,
    /// Whether an entry over the budget needs a manager (`SAL.ACL.07`).
    pub enforce: bool,
}

impl BudgetCheck {
    /// Whether an entry of `amount` takes the approved costs over the budget
    /// and so needs approval. A negative entry, a campaign without a budget
    /// and the setting off never do.
    pub fn over(&self, amount: i64) -> bool {
        self.enforce && amount > 0 && self.budget.is_some_and(|b| self.approved + amount > b)
    }
}

/// A threshold of a campaign's budget that was reached.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_campaign_budget_alerts",
    unique(campaign_id, threshold_bps)
)]
pub struct BudgetAlert {
    /// The primary key.
    pub id: i64,
    /// The campaign.
    #[model(references = "wl_sales_campaigns")]
    pub campaign_id: i64,
    /// The threshold in basis points of the budget (8000 is 80%).
    pub threshold_bps: i64,
    /// The approved costs when it was reached.
    pub reached_amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Whether `total` has reached `bps` basis points of `budget`.
pub fn reached(total: i64, budget: i64, bps: u32) -> bool {
    i128::from(total) * 10_000 >= i128::from(budget) * i128::from(bps)
}

/// Records the thresholds the approved costs of campaign `campaign_id` have
/// now reached and not reached before, under the campaign lock, then tells
/// the owner once per new row. A campaign without a budget alerts nobody.
pub async fn check_alerts(state: &AppState, campaign_id: i64) -> Result {
    let config = state
        .provided::<SalesConfig>()
        .map(|c| (*c).clone())
        .unwrap_or_default();
    let mut tx = state.db.begin().await?;
    costs::lock(&mut tx, campaign_id).await?;
    let Some(campaign) = Campaign::find(&mut tx, campaign_id).await? else {
        return Ok(());
    };
    let Some(budget) = campaign.budget.filter(|b| *b > 0) else {
        return Ok(());
    };
    let total = costs::totals(&mut tx, campaign_id, None).await?.approved;
    let mut fresh = Vec::new();
    for &bps in &config.budget_alerts {
        if !reached(total, budget, bps) {
            continue;
        }
        let at = now();
        let written = renox::db::sql(format!(
            "INSERT INTO {} (campaign_id, threshold_bps, reached_amount, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT (campaign_id, threshold_bps) DO NOTHING",
            BudgetAlert::TABLE
        ))
        .bind(campaign_id)
        .bind(i64::from(bps))
        .bind(total)
        .bind(at)
        .bind(at)
        .execute(&mut tx)
        .await?;
        if written == 1 {
            fresh.push(bps);
        }
    }
    tx.commit().await?;
    for bps in fresh {
        notices::budget_alert(state, &campaign, bps, total).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(approved: i64, budget: Option<i64>, enforce: bool) -> BudgetCheck {
        BudgetCheck {
            approved,
            budget,
            enforce,
        }
    }

    #[test]
    fn only_a_positive_entry_past_the_budget_is_over() {
        let c = check(480_000, Some(500_000), true);
        assert!(c.over(40_000));
        assert!(!c.over(20_000), "exactly the budget is within");
        assert!(!c.over(-30_000));
    }

    #[test]
    fn no_budget_or_the_setting_off_is_never_over() {
        assert!(!check(480_000, None, true).over(10_000_000));
        assert!(!check(480_000, Some(500_000), false).over(10_000_000));
    }

    #[test]
    fn a_threshold_is_reached_at_its_share_of_the_budget() {
        assert!(!reached(399_999, 500_000, 8000));
        assert!(reached(400_000, 500_000, 8000));
        assert!(reached(500_000, 500_000, 10_000));
        assert!(!reached(499_999, 500_000, 10_000));
    }
}
