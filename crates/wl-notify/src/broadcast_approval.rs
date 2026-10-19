//! The approval of a broadcast above the size limit (wl-notify §4.3, #61 A4,
//! A5): the default policy, the final re-check and the `ApprovalDecided`
//! listener.
//!
//! The policy asks one holder of `notifications.marketing.send`, never the
//! requester (segregation of duties on, so not even as admin), and lets a
//! request wait 24 hours (the kernel expiry is a fixed duration; #63 closes
//! the exact rule). The final re-check runs at the last yes before the
//! broadcast is scheduled: it checks that the broadcast and its type, channel
//! and text are still there. The requester check needs the app state, which
//! the kernel hook does not have, so it is run again by `start` (#61 part 5).
//!
//! When a request ends without a yes the kernel runs `back_to_draft` on the
//! waiting broadcast, which cancels it. The listener then fixes the stored
//! reason, emits `BroadcastCancelled` once and tells the requester.

use renox::prelude::*;
use wl_kernel::approvals::{
    ApprovalDecided, ApprovalEnd, ApprovalRequest, PolicyDefault, register_final_check,
};
use wl_kernel::broadcast::BroadcastCancelReason;

use crate::broadcast::{Broadcast, BroadcastStatus};
use crate::broadcast_machine::{ACTION, APPROVAL_EXPIRED, CANCELLED, SUBJECT_TYPE};
use crate::broadcasts::{announce_cancelled, type_problem};
use crate::permissions;
use crate::texts::english;
use crate::types::NotificationType;

/// Hours a request may wait for a decision.
pub const APPROVAL_EXPIRY_HOURS: i64 = 24;

/// The default policy: always one step by a holder of
/// `notifications.marketing.send`, never the requester.
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(ACTION)
        .step_permission(permissions::MARKETING_SEND)
        .expiry_hours(Some(APPROVAL_EXPIRY_HOURS))
}

/// Why the approval ended, as the broadcast's cancel reason; `None` for an
/// approval.
pub fn cancel_reason(end: ApprovalEnd) -> Option<BroadcastCancelReason> {
    match end {
        ApprovalEnd::Approved => None,
        ApprovalEnd::Rejected | ApprovalEnd::Returned | ApprovalEnd::Failed => {
            Some(BroadcastCancelReason::Rejected)
        }
        ApprovalEnd::Withdrawn => Some(BroadcastCancelReason::Cancelled),
        ApprovalEnd::Expired => Some(BroadcastCancelReason::ApprovalExpired),
    }
}

/// The final re-check at the last yes: `Some(message)` fails the request.
async fn recheck(
    tx: &mut renox::db::Transaction,
    request: &ApprovalRequest,
) -> Result<Option<String>> {
    let Some(broadcast) = Broadcast::query()
        .where_eq("id", request.subject_id)
        .first(&mut *tx)
        .await?
    else {
        return Ok(Some(english("wl_notify.broadcast.problem.not_found")));
    };
    let def = match NotificationType::query()
        .where_(NotificationType::KEY.eq(broadcast.type_key.as_str()))
        .first(&mut *tx)
        .await?
    {
        Some(row) => Some(row.to_def()),
        None => wl_kernel::notify::notification_type(&broadcast.type_key).map(|d| (*d).clone()),
    };
    let problem = match def {
        Some(def) => type_problem(&def, broadcast.channel),
        None => Some("wl_notify.broadcast.problem.unknown_type"),
    };
    Ok(problem.map(english))
}

/// Registers the default policy, the final re-check and the listener.
pub(crate) fn register(app: &mut Registry) {
    policy().register();
    register_final_check(SUBJECT_TYPE, |tx, request| Box::pin(recheck(tx, request)));
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        // `state_returned_to` is empty when the broadcast did not go back:
        // it was cancelled another way, which announced it.
        if event.subject.subject_type != SUBJECT_TYPE || event.state_returned_to.is_none() {
            return Ok(());
        }
        let Some(reason) = cancel_reason(event.decision) else {
            return Ok(());
        };
        ended(&state, event.subject.subject_id, reason).await
    });
}

/// A request about broadcast `id` ended without a yes: fixes the stored
/// reason of a withdrawal or an expiry, then announces the cancel.
async fn ended(state: &AppState, id: i64, reason: BroadcastCancelReason) -> Result {
    let Some(mut broadcast) = Broadcast::find(&state.db, id).await? else {
        return Ok(());
    };
    if broadcast.status != BroadcastStatus::Cancelled {
        return Ok(());
    }
    let fixed = match reason {
        BroadcastCancelReason::Cancelled => Some(CANCELLED),
        BroadcastCancelReason::ApprovalExpired => Some(APPROVAL_EXPIRED),
        _ => None,
    };
    if let Some(text) = fixed {
        Broadcast::query()
            .where_(Broadcast::ID.eq(id))
            .update(&state.db, &[("cancel_reason", &Some(text.to_owned()))])
            .await?;
        broadcast.cancel_reason = Some(text.to_owned());
    }
    announce_cancelled(state, &broadcast, reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_is_one_step_by_the_holders_with_sod_and_a_day() {
        let expected = PolicyDefault::new("notifications.broadcast_approval")
            .step_permission("notifications.marketing.send")
            .expiry_hours(Some(24));
        assert_eq!(policy(), expected);
    }

    #[test]
    fn every_end_without_a_yes_cancels_with_a_reason() {
        assert_eq!(cancel_reason(ApprovalEnd::Approved), None);
        assert_eq!(
            cancel_reason(ApprovalEnd::Rejected),
            Some(BroadcastCancelReason::Rejected)
        );
        assert_eq!(
            cancel_reason(ApprovalEnd::Withdrawn),
            Some(BroadcastCancelReason::Cancelled)
        );
        assert_eq!(
            cancel_reason(ApprovalEnd::Expired),
            Some(BroadcastCancelReason::ApprovalExpired)
        );
        assert!(cancel_reason(ApprovalEnd::Failed).is_some());
        assert!(cancel_reason(ApprovalEnd::Returned).is_some());
    }
}
