//! Asking for a broadcast (wl-notify §4.3, #61 A2, A4, A6, A7): the body of
//! `Notifier::broadcast` for [`crate::Messages`].
//!
//! [`request`] refuses (422) a request with no named user, a requester that is
//! not active (`NTF.ACL.04`), a type that is not of category `marketing`, a
//! channel the type does not use or has no text for, and a marketing SMS
//! while no `AD-` sender ID is configured. It then resolves the segment once
//! ([`crate::broadcast_recipients::resolve`], the function `start` uses too)
//! for the estimate, records the broadcast as a draft with that estimate and
//! runs `request` (estimate within the limit) or `request_over_limit` (above
//! it, an approval request) in the same transaction. The approvers are told
//! after the commit.
//!
//! [`announce_cancelled`] emits `BroadcastCancelled` and tells the requester;
//! every path that cancels a broadcast calls it once.

use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::broadcast::{
    BroadcastCancelReason, BroadcastCancelled, BroadcastId, BroadcastRequest, RequesterCheck,
    requesters,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::broadcast::{Broadcast, settings};
use crate::broadcast_machine::{REQUEST, REQUEST_OVER_LIMIT, RequestInput};
use crate::broadcast_recipients::resolve;
use crate::clock::clock;
use crate::texts::{english, english_with};
use crate::types::NotificationType;

/// The notification type of the requester's notice that a broadcast was
/// cancelled.
pub const CANCELLED_NOTICE: &str = "notify.broadcast_cancelled";

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key))
}

/// Why `def` cannot carry a marketing broadcast on `channel`, as a text key.
pub(crate) fn type_problem(
    def: &NotificationTypeDef,
    channel: NotifyChannel,
) -> Option<&'static str> {
    if def.category != Category::Marketing {
        Some("wl_notify.broadcast.problem.not_marketing")
    } else if !def.channels.contains(&channel) {
        Some("wl_notify.broadcast.problem.channel_unused")
    } else if !def.templates.iter().any(|t| t.channel == channel) {
        Some("wl_notify.broadcast.problem.no_template")
    } else {
        None
    }
}

/// The type `key` as stored, else as registered; `None` when neither.
async fn find_type(db: &Db, key: &str) -> Result<Option<NotificationTypeDef>> {
    if let Some(row) = NotificationType::by_key(db, key).await? {
        return Ok(Some(row.to_def()));
    }
    Ok(notify::notification_type(key).map(|def| (*def).clone()))
}

/// Records `request` as a broadcast and returns its id: `scheduled` when its
/// estimate is within the limit, `pending_approval` above it.
///
/// # Errors
///
/// 422 for a request that breaks one of the rules above; the database's
/// errors.
pub async fn request(state: &AppState, request: BroadcastRequest) -> Result<BroadcastId> {
    if request.requested_by <= 0 || User::find(&state.db, request.requested_by).await?.is_none() {
        return Err(refused("wl_notify.broadcast.problem.no_user"));
    }
    let at = clock(state).now();
    match requesters(state)
        .check(state, &request.requester, at)
        .await?
    {
        RequesterCheck::Active => {}
        RequesterCheck::Ended => {
            return Err(refused("wl_notify.broadcast.problem.requester_ended"));
        }
        RequesterCheck::GuardFailed(reason) => {
            return Err(Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                english_with(
                    "wl_notify.broadcast.problem.requester_refused",
                    &[("reason", &reason)],
                ),
            ));
        }
    }
    let Some(def) = find_type(&state.db, &request.type_key).await? else {
        return Err(refused("wl_notify.broadcast.problem.unknown_type"));
    };
    if let Some(problem) = type_problem(&def, request.channel) {
        return Err(refused(problem));
    }
    let settings = settings(state);
    if request.channel == NotifyChannel::Sms && settings.valid_sms_sender_id().is_none() {
        return Err(refused("wl_notify.broadcast.problem.no_sender_id"));
    }

    let mut draft = Broadcast {
        type_key: request.type_key.clone(),
        requester_type: request.requester.subject_type.clone(),
        requester_id: request.requester.subject_id,
        requested_by: request.requested_by,
        channel: request.channel,
        segment_id: request.segment.0,
        send_at: request.send_at,
        ..Default::default()
    };
    draft.estimate = resolve(state, &draft, at).await?.recipients.len() as i64;

    let name = if settings.needs_approval(draft.estimate) {
        REQUEST_OVER_LIMIT
    } else {
        REQUEST
    };
    let input = json!(RequestInput {
        limit: settings.approval_above
    });
    let mut tx = state.db.begin().await?;
    let row = Broadcast::create(&mut tx, draft).await?;
    let outcome = row
        .transition(name)
        .by_system()
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    if name == REQUEST_OVER_LIMIT
        && let Some(stored) = Broadcast::find(&state.db, row.id).await?
        && let Some(id) = stored.approval_request_id
        && let Some(approval) = ApprovalRequest::find(&state.db, id).await?
    {
        approvals::announce_request(state, &approval).await?;
    }
    Ok(row.broadcast_id())
}

/// Emits [`BroadcastCancelled`] for `broadcast` (already `cancelled`) and
/// tells the user who asked for it. The notice holds the reason, no names.
pub async fn announce_cancelled(
    state: &AppState,
    broadcast: &Broadcast,
    reason: BroadcastCancelReason,
) -> Result {
    state
        .emit(BroadcastCancelled {
            broadcast: broadcast.broadcast_id(),
            requester: broadcast.requester(),
            reason,
        })
        .await?;
    let why = broadcast
        .cancel_reason
        .clone()
        .unwrap_or_else(|| reason.as_str().to_owned());
    let notice = NotifyRequest::new(CANCELLED_NOTICE, broadcast.broadcast_id().subject())
        .to_user(broadcast.requested_by)
        .data(json!({
            "broadcast": broadcast.id,
            "channel": broadcast.channel.to_string(),
            "reason": why,
        }));
    notify::notifier(state).notify(state, notice).await?;
    Ok(())
}

/// The requester's notice type: transactional, to a user, e-mail text.
pub fn cancelled_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CANCELLED_NOTICE, "notify")
        .group("notify")
        .template(
            NotifyChannel::Email,
            "en",
            Some(&english("wl_notify.broadcast.cancelled.subject")),
            &english("wl_notify.broadcast.cancelled.body"),
        )
}

/// Registers the notice type with the kernel registry.
pub(crate) fn register_type() {
    notify::register_type(cancelled_notice_type());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> NotificationTypeDef {
        NotificationTypeDef::new("campaign.offer", "sales")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms])
            .template(NotifyChannel::Sms, "en", None, "Offer")
    }

    #[test]
    fn a_marketing_type_with_a_text_for_the_channel_passes() {
        assert_eq!(type_problem(&offer(), NotifyChannel::Sms), None);
    }

    #[test]
    fn a_type_that_is_not_marketing_is_refused() {
        let def = offer().category(Category::Transactional);
        assert_eq!(
            type_problem(&def, NotifyChannel::Sms),
            Some("wl_notify.broadcast.problem.not_marketing")
        );
    }

    #[test]
    fn a_channel_the_type_does_not_use_or_has_no_text_for_is_refused() {
        assert_eq!(
            type_problem(&offer(), NotifyChannel::Email),
            Some("wl_notify.broadcast.problem.channel_unused")
        );
        let bare = NotificationTypeDef::new("campaign.bare", "sales")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Sms]);
        assert_eq!(
            type_problem(&bare, NotifyChannel::Sms),
            Some("wl_notify.broadcast.problem.no_template")
        );
    }

    #[test]
    fn the_cancel_notice_is_a_transactional_type_with_a_text() {
        let def = cancelled_notice_type();
        assert_eq!(def.key, "notify.broadcast_cancelled");
        assert_eq!(def.category, Category::Transactional);
        assert!(def.template_for(NotifyChannel::Email, "en").is_some());
    }
}
