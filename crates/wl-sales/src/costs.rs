//! The cost entry services (wl-sales §4, #138 A12, A18): [`save`], [`edit`],
//! the campaign lock and [`totals`].
//!
//! Every budget check runs under the campaign lock: the first statement of
//! the transaction bumps `lock_version` of the campaign row, which takes the
//! write lock on SQLite and the row lock on PostgreSQL. So of two entries
//! saved at the same time, the second reads the first's approved total.

use std::future::Future;
use std::pin::Pin;

use renox::chrono::NaiveDate;
use renox::db::{Executor, Transaction, now, sql};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Outcome, StateMachine};
use wl_kernel::approvals;

use crate::budget::BudgetCheck;
use crate::campaign::Campaign;
use crate::config::SalesConfig;
use crate::cost_entry::{CostEntry, CostRefused, CostStatus};
use crate::cost_machine::{SUBMIT, SUBMIT_OVER_BUDGET};

/// What a user enters.
#[derive(Debug, Clone)]
pub struct CostInput {
    /// The campaign.
    pub campaign_id: i64,
    /// The day of the cost.
    pub entry_date: NaiveDate,
    /// Fils of AED; negative for a credit.
    pub amount: i64,
    /// What it was for.
    pub description: String,
    /// Who enters it.
    pub requested_by: i64,
    /// Whether the user holds `campaigns.costs.manage`, which a negative
    /// entry needs.
    pub manages_costs: bool,
}

/// The approved costs of a campaign and the sums that are not counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Approved costs: the only ones any total counts.
    pub approved: i64,
    /// Of them, linked to supplier bills.
    pub bill_linked: i64,
    /// Of them, entered by hand.
    pub manual: i64,
    /// Waiting for approval; not counted.
    pub pending: i64,
    /// Waiting for their bill's approval; not counted.
    pub awaiting_bill: i64,
}

/// Takes the lock of campaign `campaign_id` inside `tx`. Returns whether the
/// campaign exists.
pub async fn lock(tx: &mut Transaction, campaign_id: i64) -> Result<bool> {
    let locked = sql(format!(
        "UPDATE {} SET lock_version = lock_version + 1 WHERE id = ?",
        Campaign::TABLE
    ))
    .bind(campaign_id)
    .execute(&mut *tx)
    .await?;
    Ok(locked == 1)
}

/// The totals of campaign `campaign` for entries dated inside `period`
/// (both ends included), or for all time.
pub async fn totals<'c>(
    db: impl Executor<'c>,
    campaign: i64,
    period: Option<(NaiveDate, NaiveDate)>,
) -> Result<Totals> {
    let mut query = CostEntry::query().where_(CostEntry::CAMPAIGN_ID.eq(campaign));
    if let Some((from, to)) = period {
        query = query
            .where_(CostEntry::ENTRY_DATE.gte(from))
            .where_(CostEntry::ENTRY_DATE.lte(to));
    }
    let mut totals = Totals::default();
    for entry in query.get(db).await? {
        match entry.status {
            CostStatus::Approved => {
                totals.approved += entry.amount;
                if entry.supplier_bill_id.is_some() {
                    totals.bill_linked += entry.amount;
                } else {
                    totals.manual += entry.amount;
                }
            }
            CostStatus::PendingApproval => totals.pending += entry.amount,
            CostStatus::AwaitingBill => totals.awaiting_bill += entry.amount,
            CostStatus::Draft | CostStatus::Rejected | CostStatus::Voided => {}
        }
    }
    Ok(totals)
}

/// The delete check of `campaigns::delete`: any cost entry, in any status,
/// keeps the campaign.
pub fn delete_check<'a>(
    tx: &'a mut Transaction,
    campaign: i64,
) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
    Box::pin(async move {
        let count = CostEntry::query()
            .where_(CostEntry::CAMPAIGN_ID.eq(campaign))
            .count(&mut *tx)
            .await?;
        Ok((count > 0).then(|| crate::texts::english("wl_sales.costs.in_use", &[])))
    })
}

/// Reads the approved total under the lock already held by `tx` and runs the
/// one transition the budget decides: `submit` or `submit_over_budget`.
async fn submit_in(
    state: &AppState,
    tx: &mut Transaction,
    entry: &CostEntry,
    campaign: &Campaign,
) -> Result<Outcome<CostEntry>> {
    let enforce = state
        .provided::<SalesConfig>()
        .is_none_or(|c| c.over_budget_approval);
    let check = BudgetCheck {
        approved: totals(&mut *tx, campaign.id, None).await?.approved,
        budget: campaign.budget,
        enforce,
    };
    let name = if check.over(entry.amount) {
        SUBMIT_OVER_BUDGET
    } else {
        SUBMIT
    };
    entry
        .transition(name)
        .by_system()
        .input(&json!(check))
        .meta(json!({ "requested_by": entry.requested_by }))
        .run_in(tx)
        .await
        .map_err(|e| e.into_error())
}

/// After the commit: the approval request is announced to its approvers and
/// the engine's event is emitted.
async fn finish(state: &AppState, outcome: Outcome<CostEntry>) -> Result {
    approvals::announce(state, &outcome).await?;
    outcome.emit(state).await
}

/// Checks the parts of `input` that do not need the database.
fn check(input: &CostInput) -> Option<CostRefused> {
    if input.amount == 0 {
        Some(CostRefused::NoAmount)
    } else if input.amount < 0 && !input.manages_costs {
        Some(CostRefused::NegativeNeedsManager)
    } else {
        None
    }
}

/// Saves a manual cost entry and submits it, in one transaction under the
/// campaign lock: `approved` when it keeps the approved costs within the
/// budget (or is negative, or the campaign has no budget), `pending_approval`
/// with a kernel request when it takes them over.
pub async fn save(
    state: &AppState,
    input: CostInput,
) -> Result<std::result::Result<CostEntry, CostRefused>> {
    if let Some(refused) = check(&input) {
        return Ok(Err(refused));
    }
    let mut tx = state.db.begin().await?;
    if !lock(&mut tx, input.campaign_id).await? {
        return Ok(Err(CostRefused::CampaignNotFound));
    }
    let Some(campaign) = Campaign::find(&mut tx, input.campaign_id).await? else {
        return Ok(Err(CostRefused::CampaignNotFound));
    };
    let entry = CostEntry::create(
        &mut tx,
        CostEntry {
            campaign_id: campaign.id,
            entry_date: input.entry_date,
            amount: input.amount,
            description: input.description.trim().to_owned(),
            requested_by: input.requested_by,
            scope_type: campaign.scope_type.clone(),
            scope_id: campaign.scope_id.clone(),
            ..CostEntry::default()
        },
    )
    .await?;
    let outcome = submit_in(state, &mut tx, &entry, &campaign).await?;
    tx.commit().await?;
    finish(state, outcome).await?;
    Ok(Ok(CostEntry::find_or_404(&state.db, entry.id).await?))
}

/// Changes entry `id` as `requester` and submits it again with a new budget
/// check. A pending entry is withdrawn first (kernel `withdraw`, back to
/// `draft`). An `approved` entry is never edited; only the one who entered
/// it edits it.
pub async fn edit(
    state: &AppState,
    id: i64,
    input: CostInput,
) -> Result<std::result::Result<CostEntry, CostRefused>> {
    if let Some(refused) = check(&input) {
        return Ok(Err(refused));
    }
    let Some(entry) = CostEntry::find(&state.db, id).await? else {
        return Ok(Err(CostRefused::NotFound));
    };
    if entry.requested_by != input.requested_by {
        return Ok(Err(CostRefused::NotRequester));
    }
    match entry.status {
        CostStatus::Approved => return Ok(Err(CostRefused::Approved)),
        CostStatus::Draft | CostStatus::PendingApproval => {}
        _ => return Ok(Err(CostRefused::NotEditable)),
    }
    if entry.status == CostStatus::PendingApproval {
        let (Some(request), Some(user)) = (
            entry.approval_request_id,
            User::find(&state.db, input.requested_by).await?,
        ) else {
            return Ok(Err(CostRefused::NotEditable));
        };
        approvals::withdraw(state, request, &user).await?;
    }
    let mut tx = state.db.begin().await?;
    if !lock(&mut tx, entry.campaign_id).await? {
        return Ok(Err(CostRefused::CampaignNotFound));
    }
    let Some(campaign) = Campaign::find(&mut tx, entry.campaign_id).await? else {
        return Ok(Err(CostRefused::CampaignNotFound));
    };
    // Read again under the lock: the withdraw above has moved it.
    let Some(entry) = CostEntry::find(&mut tx, id).await? else {
        return Ok(Err(CostRefused::NotFound));
    };
    if entry.status != CostStatus::Draft {
        return Ok(Err(CostRefused::NotEditable));
    }
    let description = input.description.trim().to_owned();
    CostEntry::query()
        .where_(CostEntry::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("entry_date", &input.entry_date),
                ("amount", &input.amount),
                ("description", &description),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let entry = CostEntry::find_or_404(&mut tx, id).await?;
    let outcome = submit_in(state, &mut tx, &entry, &campaign).await?;
    tx.commit().await?;
    finish(state, outcome).await?;
    Ok(Ok(CostEntry::find_or_404(&state.db, id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(amount: i64, manages_costs: bool) -> CostInput {
        CostInput {
            campaign_id: 1,
            entry_date: NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
            amount,
            description: "Ads".into(),
            requested_by: 2,
            manages_costs,
        }
    }

    #[test]
    fn a_zero_amount_and_an_unauthorised_credit_are_refused() {
        assert_eq!(check(&input(0, true)), Some(CostRefused::NoAmount));
        assert_eq!(
            check(&input(-100, false)),
            Some(CostRefused::NegativeNeedsManager)
        );
        assert_eq!(check(&input(-100, true)), None);
        assert_eq!(check(&input(100, false)), None);
    }
}
