//! Stale broadcasts and stopping a broadcast (wl-notify §4.3, §6 `NTF.RUL.03`,
//! #63).
//!
//! The scheduled task [`TASK`] runs [`run`] every 15 minutes:
//!
//! - [`expire_unapproved`] ends the approval request of a broadcast still
//!   `pending_approval` [`StaleSettings::approval_expiry_hours`] (24) after
//!   its send time. The kernel runs `back_to_draft` on the broadcast, which
//!   cancels it; the approval listener ([`crate::broadcast_approval`]) fixes
//!   the reason to *approval expired*, tells the requester and emits
//!   `BroadcastCancelled` once. A broadcast approved before the check is not
//!   pending any more and is left alone; an approval and the expiry at the
//!   same moment: the request is locked, the first commit wins.
//! - [`suppress_stale`] runs `suppress` with the reason [`EXPIRED`] on each
//!   message of a broadcast still `queued` or `deferred`
//!   [`StaleSettings::stale_hours`] (48) after the broadcast's `start`. The
//!   limit is measured from `start`, not from a message's deferral. The send
//!   itself refuses such a message too ([`send_guard`]), so a provider that
//!   recovers late sends nothing of it, and a `suppressed` message is never
//!   retried.
//!
//! [`stop`] is the action of a holder of `notifications.marketing.send` on a
//! broadcast that is `sending`, or `completed` with messages still `queued` or
//! `deferred`: those messages are `suppressed` with [`BROADCAST_STOPPED`], the
//! state stays, who and when are stored, `BroadcastStopped` is emitted and the
//! requester is told. A stopped `sending` broadcast queues no more messages.
//!
//! Stale and stopped messages are excluded counts of the broadcast
//! ([`crate::broadcast_results::excluded_with_cap`]) and never count towards
//! the frequency cap (only `sent_at` messages do).

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::approvals;
use wl_kernel::broadcast::{BroadcastCancelReason, BroadcastStopped};
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::broadcast::{Broadcast, BroadcastStatus};
use crate::broadcast_machine::EXPIRE;
use crate::broadcast_run::lock;
use crate::broadcasts::announce_cancelled;
use crate::clock::clock;
use crate::machine::SUPPRESS;
use crate::message::{Message, MessageStatus};
use crate::permissions;
use crate::send::run as run_message;
use crate::texts::english;

/// The scheduled task that runs [`run`] every 15 minutes.
pub const TASK: &str = "notify:stale_broadcasts";

/// The reason of a message suppressed because its broadcast went stale.
pub const EXPIRED: &str = "expired";

/// The reason of a message suppressed because its broadcast was stopped.
pub const BROADCAST_STOPPED: &str = "broadcast_stopped";

/// The notification type of the requester's notice of a stop.
pub const STOPPED_NOTICE: &str = "notify.broadcast_stopped";

/// Hours after its send time an unapproved broadcast expires, unless set.
pub const DEFAULT_APPROVAL_EXPIRY_HOURS: i64 = 24;

/// Hours after its start a queued or deferred message is stale, unless set.
pub const DEFAULT_STALE_HOURS: i64 = 48;

/// The module's `NTF.RUL.03` settings, set on [`WlNotify`](crate::WlNotify)
/// with `broadcast_expiry_hours` and `stale_message_hours`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleSettings {
    /// N: hours after send at a broadcast may wait for its approval.
    pub approval_expiry_hours: i64,
    /// M: hours after start a marketing message may stay unsent.
    pub stale_hours: i64,
}

impl Default for StaleSettings {
    fn default() -> Self {
        StaleSettings {
            approval_expiry_hours: DEFAULT_APPROVAL_EXPIRY_HOURS,
            stale_hours: DEFAULT_STALE_HOURS,
        }
    }
}

/// The app's settings, or the defaults when none are provided.
pub fn settings(state: &AppState) -> StaleSettings {
    state
        .provided::<StaleSettings>()
        .map(|s| *s)
        .unwrap_or_default()
}

/// What one [`run`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Swept {
    /// Broadcasts cancelled because nobody approved them in time.
    pub expired: usize,
    /// Messages suppressed as stale.
    pub suppressed: usize,
}

/// The sweep: expires unapproved broadcasts, then suppresses stale messages,
/// as of `now`.
pub async fn run(state: &AppState, now: DateTime) -> Result<Swept> {
    Ok(Swept {
        expired: expire_unapproved(state, now).await?,
        suppressed: suppress_stale(state, now).await?,
    })
}

/// Cancels every broadcast still `pending_approval` when its send time is
/// `approval_expiry_hours` or more behind `now`. Returns how many.
pub async fn expire_unapproved(state: &AppState, now: DateTime) -> Result<usize> {
    let limit = now - TimeDelta::hours(settings(state).approval_expiry_hours);
    let due = Broadcast::query()
        .where_(Broadcast::STATUS.eq(BroadcastStatus::PendingApproval))
        .where_(Broadcast::SEND_AT.lte(limit))
        .order_by(Broadcast::ID)
        .get(&state.db)
        .await?;
    let mut expired = 0;
    for broadcast in due {
        let ended = match broadcast.approval_request_id {
            Some(request) => end_request(state, &broadcast, request).await?,
            None => expire_direct(state, broadcast.id).await?,
        };
        if ended {
            expired += 1;
        }
    }
    Ok(expired)
}

/// Ends approval request `request` of `broadcast` as expired. `false`
/// when an approval (or a rejection) got there first.
///
/// Decision: the request has no expiry of its own (see
/// [`crate::broadcast_approval::policy`]) and the kernel only expires a
/// request past its `expires_at`, so the limit of `NTF.RUL.03` (send time
/// plus [`StaleSettings::approval_expiry_hours`]) is stored on the request
/// just before it is expired.
async fn end_request(state: &AppState, broadcast: &Broadcast, request: i64) -> Result<bool> {
    let id = broadcast.id;
    let limit = broadcast.send_at + TimeDelta::hours(settings(state).approval_expiry_hours);
    approvals::ApprovalRequest::query()
        .where_(approvals::ApprovalRequest::ID.eq(request))
        .where_(approvals::ApprovalRequest::STATUS.eq(approvals::ApprovalStatus::Pending))
        .update(&state.db, &[("expires_at", &Some(limit))])
        .await?;
    match approvals::expire(state, request).await {
        Ok(_) => Ok(true),
        Err(error) => {
            let current = Broadcast::find(&state.db, id).await?;
            if current.is_some_and(|b| b.status == BroadcastStatus::PendingApproval) {
                Err(error)
            } else {
                Ok(false)
            }
        }
    }
}

/// `expire` for a broadcast with no approval request to end.
async fn expire_direct(state: &AppState, id: i64) -> Result<bool> {
    let mut tx = state.db.begin().await?;
    let Some(locked) = lock(&mut tx, id).await? else {
        return Ok(false);
    };
    if locked.status != BroadcastStatus::PendingApproval {
        return Ok(false);
    }
    let outcome = locked
        .transition(EXPIRE)
        .by_system()
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let cancelled = Broadcast::find_or_404(&state.db, id).await?;
    announce_cancelled(state, &cancelled, BroadcastCancelReason::ApprovalExpired).await?;
    Ok(true)
}

/// The messages of broadcast `id` still `queued` or `deferred`, read through
/// `db`.
async fn unsent<'c>(db: impl renox::db::Executor<'c>, id: i64) -> Result<Vec<Message>> {
    Message::query()
        .where_(Message::BROADCAST_ID.eq(id))
        .where_(Message::STATUS.is_in([MessageStatus::Queued, MessageStatus::Deferred]))
        .order_by(Message::ID)
        .get(db)
        .await
}

/// Whether `by` may stop `broadcast` now: a holder of
/// `notifications.marketing.send`, and the broadcast is `sending` (not stopped
/// yet) or `completed` with messages still `queued` or `deferred`.
pub async fn may_stop(db: &Db, by: &renox::AuthUser, broadcast: &Broadcast) -> Result<bool> {
    if !by.has_permission(permissions::MARKETING_SEND) {
        return Ok(false);
    }
    match broadcast.status {
        BroadcastStatus::Sending if broadcast.stopped_at.is_none() => Ok(true),
        BroadcastStatus::Sending | BroadcastStatus::Completed => {
            Message::query()
                .where_(Message::BROADCAST_ID.eq(broadcast.id))
                .where_(Message::STATUS.is_in([MessageStatus::Queued, MessageStatus::Deferred]))
                .exists(db)
                .await
        }
        _ => Ok(false),
    }
}

/// Suppresses, with [`EXPIRED`], every message still `queued` or `deferred`
/// of a broadcast that started `stale_hours` or more before `now`. Returns
/// how many.
pub async fn suppress_stale(state: &AppState, now: DateTime) -> Result<usize> {
    let limit = now - TimeDelta::hours(settings(state).stale_hours);
    let started = Broadcast::query()
        .where_(Broadcast::STATUS.is_in([BroadcastStatus::Sending, BroadcastStatus::Completed]))
        .where_(Broadcast::STARTED_AT.lte(limit))
        .order_by(Broadcast::ID)
        .get(&state.db)
        .await?;
    let mut suppressed = 0;
    for broadcast in started {
        for message in unsent(&state.db, broadcast.id).await? {
            let mut tx = state.db.begin().await?;
            // Read again inside the transaction: a send may have got there.
            let Some(current) = Message::find(&mut tx, message.id).await? else {
                continue;
            };
            if !matches!(
                current.status,
                MessageStatus::Queued | MessageStatus::Deferred
            ) {
                continue;
            }
            let outcome =
                run_message(&current, SUPPRESS, Some(EXPIRED.to_owned()), &mut tx).await?;
            tx.commit().await?;
            outcome.emit(state).await?;
            suppressed += 1;
        }
    }
    Ok(suppressed)
}

/// The `send` guard of a broadcast message (`NTF.RUL.03`): one whose
/// broadcast was stopped, or started `stale_hours` or more before `now`, is
/// `suppressed` and not sent, whatever route brought it back to `queued`
/// (a release, a retry).
pub(crate) async fn send_guard(
    state: &AppState,
    message: &Message,
    now: DateTime,
) -> Result<Option<Outcome<Message>>> {
    let Some(id) = message.broadcast_id else {
        return Ok(None);
    };
    let Some(broadcast) = Broadcast::find(&state.db, id).await? else {
        return Ok(None);
    };
    let limit = TimeDelta::hours(settings(state).stale_hours);
    let reason = if broadcast.stopped_at.is_some() {
        BROADCAST_STOPPED
    } else if broadcast.started_at.is_some_and(|at| at + limit <= now) {
        EXPIRED
    } else {
        return Ok(None);
    };
    let mut tx = state.db.begin().await?;
    let outcome = run_message(message, SUPPRESS, Some(reason.to_owned()), &mut tx).await?;
    tx.commit().await?;
    Ok(Some(outcome))
}

/// A 422 with the English text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key))
}

/// Stops broadcast `id` for `by`, a holder of `notifications.marketing.send`
/// (anyone else gets 403). The broadcast must be `sending`, or `completed`
/// with messages still `queued` or `deferred`; otherwise 422 *nothing left to
/// stop*. Returns how many messages it suppressed.
///
/// Decision: a `sending` broadcast with nothing unsent yet can be stopped once
/// (it then queues no more messages); stopping it again is *nothing left to
/// stop*.
pub async fn stop(state: &AppState, id: i64, by: &renox::AuthUser) -> Result<usize> {
    if !by.has_permission(permissions::MARKETING_SEND) {
        return Err(Error::Forbidden);
    }
    let mut tx = state.db.begin().await?;
    let Some(locked) = lock(&mut tx, id).await? else {
        return Err(Error::NotFound);
    };
    if !matches!(
        locked.status,
        BroadcastStatus::Sending | BroadcastStatus::Completed
    ) {
        return Err(refused("wl_notify.broadcast.problem.cannot_stop"));
    }
    let waiting = unsent(&mut tx, id).await?;
    let nothing_left = locked.status == BroadcastStatus::Completed || locked.stopped_at.is_some();
    if waiting.is_empty() && nothing_left {
        return Err(refused("wl_notify.broadcast.problem.nothing_left"));
    }
    let now = clock(state).now();
    Broadcast::query()
        .where_(Broadcast::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("stopped_by", &Some(by.id)),
                ("stopped_at", &Some(now)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let mut outcomes = Vec::with_capacity(waiting.len());
    for message in &waiting {
        let reason = Some(BROADCAST_STOPPED.to_owned());
        outcomes.push(run_message(message, SUPPRESS, reason, &mut tx).await?);
    }
    tx.commit().await?;
    let suppressed = outcomes.len();
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    let broadcast = Broadcast::find_or_404(&state.db, id).await?;
    state
        .emit(BroadcastStopped {
            broadcast: broadcast.broadcast_id(),
            requester: broadcast.requester(),
            stopped_by: by.id,
            suppressed: suppressed as i64,
        })
        .await?;
    let notice = NotifyRequest::new(STOPPED_NOTICE, broadcast.broadcast_id().subject())
        .to_user(broadcast.requested_by)
        .data(json!({
            "broadcast": broadcast.id,
            "channel": broadcast.channel.to_string(),
            "suppressed": suppressed,
        }));
    notify::notifier(state).notify(state, notice).await?;
    Ok(suppressed)
}

/// The requester's notice type of a stop: transactional, to a user, e-mail.
pub fn stopped_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STOPPED_NOTICE, "notify")
        .group("notify")
        .template(
            NotifyChannel::Email,
            "en",
            Some(&english("wl_notify.broadcast.stopped.subject")),
            &english("wl_notify.broadcast.stopped.body"),
        )
}

/// Registers the notice type with the kernel registry.
pub(crate) fn register_type() {
    notify::register_type(stopped_notice_type());
}

/// Registers the [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .every_minutes(15, TASK, |state: AppState| async move {
            let now = clock(&state).now();
            run(&state, now).await.map(|_| ())
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_24_hours_after_send_at_and_48_after_start() {
        let settings = StaleSettings::default();
        assert_eq!(settings.approval_expiry_hours, 24);
        assert_eq!(settings.stale_hours, 48);
        assert_eq!(TASK, "notify:stale_broadcasts");
    }

    #[test]
    fn the_stop_notice_is_a_transactional_type_with_a_text() {
        let def = stopped_notice_type();
        assert_eq!(def.key, "notify.broadcast_stopped");
        assert!(def.template_for(NotifyChannel::Email, "en").is_some());
    }
}
