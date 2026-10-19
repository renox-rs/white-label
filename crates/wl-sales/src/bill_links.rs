//! Cost entries linked to supplier bill lines (wl-sales §4, #138 A14).
//!
//! An entry may point at a line of a supplier bill, read through the
//! kernel's [`SupplierBills`](wl_kernel::invoicing::SupplierBills). Its
//! amount defaults to what is left of the line and may be lower, so one line
//! can be split over several campaigns; the entries of a line that are not
//! `rejected` or `voided` may not add up to more than the line. The check
//! runs under the campaign lock and, on PostgreSQL, locks the line's
//! existing entries too.
//!
//! A linked entry whose bill is not approved is `awaiting_bill` and counts
//! nowhere. `SupplierBillApproved` runs `bill_approved` and then the budget
//! submit for each of them, in a transaction per entry under its campaign
//! lock; `SupplierBillVoided` voids the ones in `awaiting_bill` or `draft`
//! with `void_with_bill` and tells the campaign's owner.

use renox::chrono::NaiveDate;
use renox::db::{Executor, Transaction};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::invoicing::{
    SupplierBillApproved, SupplierBillLine, SupplierBillLineId, SupplierBillStatus,
    SupplierBillVoided, supplier_bills,
};

use crate::campaign::Campaign;
use crate::cost_entry::{CostEntry, CostRefused, CostStatus};
use crate::cost_machine::{BILL_APPROVED, VOID_WITH_BILL};
use crate::{costs, notices};

/// What a user enters to link a cost to a supplier bill line.
#[derive(Debug, Clone)]
pub struct BillLinkInput {
    /// The campaign.
    pub campaign_id: i64,
    /// The day of the cost.
    pub entry_date: NaiveDate,
    /// The supplier bill line.
    pub line: SupplierBillLineId,
    /// Fils of AED; empty means what is left of the line.
    pub amount: Option<i64>,
    /// What it was for; empty means the line's description.
    pub description: Option<String>,
    /// Who links it; also the requester of any approval.
    pub requested_by: i64,
}

/// Fils of line `line_id` already taken by entries that are not `rejected`
/// or `voided`, leaving out entry `except`.
pub async fn line_used<'c>(
    db: impl Executor<'c>,
    line_id: i64,
    except: Option<i64>,
) -> Result<i64> {
    let entries = CostEntry::query()
        .where_(CostEntry::SUPPLIER_BILL_LINE_ID.eq(line_id))
        .lock_for_update()
        .get(db)
        .await?;
    Ok(entries
        .iter()
        .filter(|e| Some(e.id) != except)
        .filter(|e| !matches!(e.status, CostStatus::Rejected | CostStatus::Voided))
        .map(|e| e.amount)
        .sum())
}

/// What is left of `line` for new entries.
pub async fn line_remaining<'c>(db: impl Executor<'c>, line: &SupplierBillLine) -> Result<i64> {
    Ok(line.amount_aed - line_used(db, line.line.0, None).await?)
}

/// The line `entry` is linked to; `None` for a manual entry or a missing line.
pub(crate) async fn line_of(
    state: &AppState,
    entry: &CostEntry,
) -> Result<Option<SupplierBillLine>> {
    match entry.supplier_bill_line_id {
        Some(id) => supplier_bills(state).line(&state.db, id.into()).await,
        None => Ok(None),
    }
}

/// The checks of an edit of a linked entry, under the campaign lock: the
/// amount stays above zero and within what is left of the line.
pub(crate) async fn check_edit(
    tx: &mut Transaction,
    line: Option<&SupplierBillLine>,
    entry: &CostEntry,
    amount: i64,
) -> Result<Option<CostRefused>> {
    let Some(line_id) = entry.supplier_bill_line_id else {
        return Ok(None);
    };
    if amount <= 0 {
        return Ok(Some(CostRefused::NotPositive));
    }
    let Some(line) = line else {
        return Ok(Some(CostRefused::LineNotFound));
    };
    let used = line_used(&mut *tx, line_id, Some(entry.id)).await?;
    Ok((used + amount > line.amount_aed).then_some(CostRefused::LineExceeded))
}

/// Saves a cost linked to a supplier bill line. Under the campaign lock it
/// checks the line's remaining amount, then either submits the entry like
/// [`costs::save`] (the bill is approved) or leaves it `awaiting_bill`.
pub async fn save_linked(
    state: &AppState,
    input: BillLinkInput,
) -> Result<std::result::Result<CostEntry, CostRefused>> {
    if input.amount.is_some_and(|a| a <= 0) {
        return Ok(Err(CostRefused::NotPositive));
    }
    // Read before the transaction: the provider uses its own connection.
    let Some(line) = supplier_bills(state).line(&state.db, input.line).await? else {
        return Ok(Err(CostRefused::LineNotFound));
    };
    if line.bill_status == SupplierBillStatus::Void {
        return Ok(Err(CostRefused::BillVoided));
    }
    let mut tx = state.db.begin().await?;
    if !costs::lock(&mut tx, input.campaign_id).await? {
        return Ok(Err(CostRefused::CampaignNotFound));
    }
    let Some(campaign) = Campaign::find(&mut tx, input.campaign_id).await? else {
        return Ok(Err(CostRefused::CampaignNotFound));
    };
    let remaining = line.amount_aed - line_used(&mut tx, line.line.0, None).await?;
    let amount = input.amount.unwrap_or(remaining);
    if amount <= 0 || amount > remaining {
        return Ok(Err(CostRefused::LineExceeded));
    }
    let description = input
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map_or_else(|| line.description.clone(), str::to_owned);
    let entry = CostEntry::create(
        &mut tx,
        CostEntry {
            campaign_id: campaign.id,
            entry_date: input.entry_date,
            amount,
            description,
            supplier_bill_id: Some(line.bill.0),
            supplier_bill_line_id: Some(line.line.0),
            linked_by: Some(input.requested_by),
            requested_by: input.requested_by,
            scope_type: campaign.scope_type.clone(),
            scope_id: campaign.scope_id.clone(),
            ..CostEntry::default()
        },
    )
    .await?;
    let await_bill = !line.bill_status.is_approved();
    let outcome = costs::submit_in(state, &mut tx, &entry, &campaign, await_bill).await?;
    tx.commit().await?;
    costs::finish(state, outcome).await?;
    Ok(Ok(CostEntry::find_or_404(&state.db, entry.id).await?))
}

/// `SupplierBillApproved`: each `awaiting_bill` entry of the bill becomes
/// `draft` and is submitted, in its own transaction under the campaign lock.
pub async fn bill_approved(state: &AppState, bill: i64) -> Result {
    let waiting = CostEntry::query()
        .where_(CostEntry::SUPPLIER_BILL_ID.eq(bill))
        .where_(CostEntry::STATUS.eq(CostStatus::AwaitingBill))
        .order_by(CostEntry::ID)
        .get(&state.db)
        .await?;
    let mut first_error = None;
    for entry in waiting {
        if let Err(error) = release(state, entry).await {
            eprintln!("sales: releasing a cost entry of bill {bill} failed: {error}");
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn release(state: &AppState, entry: CostEntry) -> Result {
    let mut tx = state.db.begin().await?;
    if !costs::lock(&mut tx, entry.campaign_id).await? {
        return Ok(());
    }
    // Read again under the lock: another run may have moved it.
    let Some(entry) = CostEntry::find(&mut tx, entry.id).await? else {
        return Ok(());
    };
    if entry.status != CostStatus::AwaitingBill {
        return Ok(());
    }
    let Some(campaign) = Campaign::find(&mut tx, entry.campaign_id).await? else {
        return Ok(());
    };
    let released = entry
        .transition(BILL_APPROVED)
        .by_system()
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    let entry = CostEntry::find_or_404(&mut tx, entry.id).await?;
    let submitted = costs::submit_in(state, &mut tx, &entry, &campaign, false).await?;
    tx.commit().await?;
    released.emit(state).await?;
    costs::finish(state, submitted).await
}

/// `SupplierBillVoided`: each linked entry in `awaiting_bill` or `draft` is
/// voided with the bill, and the campaign's owner is told.
pub async fn bill_voided(state: &AppState, bill: i64) -> Result {
    let open = CostEntry::query()
        .where_(CostEntry::SUPPLIER_BILL_ID.eq(bill))
        .order_by(CostEntry::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|e| matches!(e.status, CostStatus::AwaitingBill | CostStatus::Draft));
    let mut first_error = None;
    for entry in open {
        if let Err(error) = void_with_bill(state, entry.id).await {
            eprintln!("sales: voiding a cost entry of bill {bill} failed: {error}");
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn void_with_bill(state: &AppState, id: i64) -> Result {
    let mut tx = state.db.begin().await?;
    let Some(entry) = CostEntry::find(&mut tx, id).await? else {
        return Ok(());
    };
    if !costs::lock(&mut tx, entry.campaign_id).await? {
        return Ok(());
    }
    let Some(entry) = CostEntry::find(&mut tx, id).await? else {
        return Ok(());
    };
    if !matches!(entry.status, CostStatus::AwaitingBill | CostStatus::Draft) {
        return Ok(());
    }
    let outcome = entry
        .transition(VOID_WITH_BILL)
        .by_system()
        .reason(crate::texts::english(
            "wl_sales.costs.bill_voided_reason",
            &[],
        ))
        .run_in(&mut tx)
        .await
        .map_err(|e| e.into_error())?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let entry = CostEntry::find_or_404(&state.db, id).await?;
    notices::cost_bill_voided(state, &entry).await
}

/// Registers the listeners on the supplier bill events.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: SupplierBillApproved, state: AppState| async move {
        bill_approved(&state, event.bill.0).await
    });
    app.listen(|event: SupplierBillVoided, state: AppState| async move {
        bill_voided(&state, event.bill.0).await
    });
}
