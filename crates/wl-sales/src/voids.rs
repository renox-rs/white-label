//! Voiding an entered-in-error cost entry (wl-sales §4, #138 A15), through
//! the kernel's `Voidable` (#27).
//!
//! The entry registers with the permission `campaigns.costs.manage` and the
//! campaign's branch as scope, and no reverse handlers: nothing is posted, so
//! no approval is needed. `void` / `void_approved` leave `approved` or
//! `pending_approval` and need a reason; the kernel writes the Void record
//! and emits `RecordVoided`. The module also writes the audit record
//! `sales.campaign_cost_voided`, so the void shows in the activity log, and a
//! voided pending entry's approval request is ended (withdrawn) in the same
//! transaction.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::Transition;
use wl_kernel::approvals::{self, End};
use wl_kernel::voids::{RecordVoided, Voidable};

use crate::campaign::Campaign;
use crate::cost_entry::{CostEntry, CostStatus};
use crate::cost_machine::SUBJECT_TYPE;
use crate::permissions::CAMPAIGNS_COSTS_MANAGE;

/// The audit action of a void.
pub const AUDIT_ACTION: &str = "sales.campaign_cost_voided";

/// Adds to a void transition the ending of the entry's pending approval
/// request, in the void's transaction. The status is already `voided`, so the
/// kernel only records the end and does not move the entry.
pub(crate) fn with_request_ended(transition: Transition<CostEntry>) -> Transition<CostEntry> {
    transition.effect(|e: &CostEntry, _ctx, tx| {
        let pending = (e.status == CostStatus::PendingApproval)
            .then_some(e.approval_request_id)
            .flatten();
        let requester = e.requested_by;
        Box::pin(async move {
            if let Some(request) = pending {
                // The approvers are not told: the kernel's announce needs
                // the commit, which a transition effect cannot wait for.
                let _ended = approvals::end_in(&mut *tx, request, End::withdraw(requester)).await?;
            }
            Ok(())
        })
    })
}

/// The `Voidable` entry of the cost entry.
pub fn voidable() -> Voidable {
    Voidable::of::<CostEntry>(SUBJECT_TYPE)
        .permission(CAMPAIGNS_COSTS_MANAGE)
        .scope(|e: &CostEntry| e.scope())
        .build()
}

/// Registers the entry and the audit listener.
pub(crate) fn register(app: &mut Registry) {
    voidable().register();
    app.listen(|event: RecordVoided, state: AppState| async move {
        if event.subject.subject_type != SUBJECT_TYPE {
            return Ok(());
        }
        audit(&state, &event).await
    });
}

/// Writes the audit record of a void.
async fn audit(state: &AppState, event: &RecordVoided) -> Result {
    let Some(entry) = CostEntry::find(&state.db, event.subject.subject_id).await? else {
        return Ok(());
    };
    let campaign = Campaign::find(&state.db, entry.campaign_id).await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(AUDIT_ACTION)
            .user(event.user_id)
            .subject(CostEntry::TABLE, entry.id)
            .data(json!({
                "campaign_id": entry.campaign_id,
                "campaign_code": campaign.map(|c| c.code),
                "amount": entry.amount,
                "state_before": event.state_before,
                "reason": event.reason,
                "approval_request_id": event.approval_request_id,
            })),
    )
    .await?;
    Ok(())
}
