//! Starting and completing broadcasts (wl-notify §4.3, #61 A9).
//!
//! The scheduled task [`TASK`] runs [`run_due`] every minute. It takes each
//! `scheduled` broadcast whose send time has passed, and each one still
//! `sending` (a run that stopped half way), and for each:
//!
//! 1. under the broadcast lock (a compare-and-swap on `lock_version`, so two
//!    runs never work on one broadcast), a `scheduled` broadcast asks its
//!    requester again ([`wl_kernel::broadcast::BroadcastRequesters`]). One
//!    that ended or fails a guard is cancelled with `requester_guard_failed`,
//!    the requester is told and nothing is queued. Otherwise `start` moves
//!    it to `sending`;
//! 2. the segment is resolved again ([`crate::broadcast_recipients::resolve`],
//!    the function the estimate used) and a message is queued for each
//!    recipient that has none yet, in batches of [`BATCH_SIZE`], one
//!    transaction each. The unique (broadcast, party) index and the check
//!    before each insert make a retry queue only the rest;
//! 3. when every recipient has a message, `complete` stores the counts and
//!    [`BroadcastCompleted`] is emitted once (the transition leaves
//!    `sending` only once).
//!
//! [`cancel`] is the cancel by the requester or a holder of
//! `notifications.marketing.send`.

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::approvals;
use wl_kernel::broadcast::{BroadcastCancelReason, BroadcastCompleted, RequesterCheck, requesters};
use wl_kernel::notify::NotifyRecipient;

use crate::broadcast::{Broadcast, BroadcastStatus, ExcludedCounts};
use crate::broadcast_machine::{
    CANCEL, CANCEL_SYSTEM, COMPLETE, CompleteInput, START, StartInput,
};
use crate::broadcast_recipients::{Recipient, resolve};
use crate::broadcast_results;
use crate::broadcasts::announce_cancelled;
use crate::clock::clock;
use crate::message::Message;
use crate::notifier::{compose, type_def};
use crate::opt_out;
use crate::recipients::{Resolved, Route};
use crate::send::SendMessage;

/// The scheduled task that runs [`run_due`] every minute.
pub const TASK: &str = "notify:broadcasts";

/// Messages queued in one transaction.
pub const BATCH_SIZE: usize = 100;

/// The stored reason of a broadcast cancelled because its requester ended.
pub const REQUESTER_GUARD_FAILED: &str = "requester_guard_failed";

/// How a run queues messages. The default is what the task uses; a test
/// shrinks the batch or stops after some batches to act as a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batching {
    /// Messages per transaction.
    pub size: usize,
    /// Stop after this many batches (a simulated crash); `None` to finish.
    pub max_batches: Option<usize>,
}

impl Default for Batching {
    fn default() -> Self {
        Batching {
            size: BATCH_SIZE,
            max_batches: None,
        }
    }
}

/// What a run did to one broadcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// Another run holds the broadcast, or it is not due.
    Skipped,
    /// Cancelled because its requester ended or a guard failed.
    Cancelled,
    /// Stopped before every recipient had a message.
    Partial,
    /// Every recipient has a message and `complete` ran.
    Completed,
}

/// Starts and completes every broadcast due at `now`. Returns how many it
/// completed or cancelled.
pub async fn run_due(state: &AppState, now: DateTime) -> Result<usize> {
    run_due_with(state, now, Batching::default()).await
}

/// As [`run_due`], queueing as `batching` says.
pub async fn run_due_with(state: &AppState, now: DateTime, batching: Batching) -> Result<usize> {
    let due = Broadcast::query()
        .where_(Broadcast::STATUS.is_in([BroadcastStatus::Scheduled, BroadcastStatus::Sending]))
        .where_(Broadcast::SEND_AT.lte(now))
        .order_by(Broadcast::ID)
        .get(&state.db)
        .await?;
    let mut done = 0;
    for broadcast in due {
        match run_one(state, broadcast.id, now, batching).await? {
            Ran::Completed | Ran::Cancelled => done += 1,
            Ran::Skipped | Ran::Partial => {}
        }
    }
    Ok(done)
}

/// Takes the broadcast lock inside `tx` and returns the row as it is now;
/// `None` when another run took it first or the row is gone.
pub(crate) async fn lock(tx: &mut renox::db::Transaction, id: i64) -> Result<Option<Broadcast>> {
    let Some(current) = Broadcast::find(&mut *tx, id).await? else {
        return Ok(None);
    };
    let next = current.lock_version + 1;
    let taken = Broadcast::query()
        .where_(Broadcast::ID.eq(id))
        .where_(Broadcast::LOCK_VERSION.eq(current.lock_version))
        .update(&mut *tx, &[("lock_version", &next)])
        .await?;
    Ok((taken == 1).then_some(Broadcast {
        lock_version: next,
        ..current
    }))
}

/// Runs broadcast `id` as far as `batching` lets it.
pub async fn run_one(state: &AppState, id: i64, now: DateTime, batching: Batching) -> Result<Ran> {
    let Some(seen) = Broadcast::find(&state.db, id).await? else {
        return Ok(Ran::Skipped);
    };
    if seen.send_at > now {
        return Ok(Ran::Skipped);
    }
    if seen.status == BroadcastStatus::Scheduled {
        // The provider may use the database, so ask before the transaction.
        let check = requesters(state)
            .check(state, &seen.requester(), now)
            .await?;
        match start(state, id, now, !matches!(check, RequesterCheck::Active)).await? {
            Started::Skipped => return Ok(Ran::Skipped),
            Started::Cancelled => return Ok(Ran::Cancelled),
            Started::Sending => {}
        }
    } else if seen.status != BroadcastStatus::Sending {
        return Ok(Ran::Skipped);
    }
    send_all(state, id, now, batching).await
}

enum Started {
    Skipped,
    Cancelled,
    Sending,
}

/// `start`, or `cancel_system` when the requester check failed.
async fn start(
    state: &AppState,
    id: i64,
    now: DateTime,
    requester_failed: bool,
) -> Result<Started> {
    let mut tx = state.db.begin().await?;
    let Some(locked) = lock(&mut tx, id).await? else {
        return Ok(Started::Skipped);
    };
    if locked.status != BroadcastStatus::Scheduled {
        return Ok(Started::Skipped);
    }
    let run = if requester_failed {
        locked
            .transition(CANCEL_SYSTEM)
            .by_system()
            .reason(REQUESTER_GUARD_FAILED)
    } else {
        locked
            .transition(START)
            .by_system()
            .input(&json!(StartInput { at: now }))
    };
    let outcome = run
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    if requester_failed {
        let cancelled = Broadcast::find_or_404(&state.db, id).await?;
        announce_cancelled(
            state,
            &cancelled,
            BroadcastCancelReason::RequesterGuardFailed,
        )
        .await?;
        Ok(Started::Cancelled)
    } else {
        Ok(Started::Sending)
    }
}

/// The queued message to `recipient` for `broadcast`.
fn message_for(
    def: &wl_kernel::notify::NotificationTypeDef,
    broadcast: &Broadcast,
    recipient: &Recipient,
) -> Message {
    let resolved = Resolved {
        chosen: None,
        language: recipient.language.clone(),
        route: Route {
            channel: broadcast.channel,
            address: Some(recipient.address.clone()),
            contact_point: Some(recipient.contact_point),
            suppress: None,
        },
    };
    let to = NotifyRecipient::Party {
        id: recipient.party,
    };
    let subject: Subject = broadcast.broadcast_id().subject();
    let data = json!({ "broadcast": broadcast.id });
    let mut message = compose(def, &subject, &to, &resolved, &data);
    message.broadcast_id = Some(broadcast.id);
    message
}

/// Queues the missing messages in batches, then completes.
async fn send_all(state: &AppState, id: i64, now: DateTime, batching: Batching) -> Result<Ran> {
    let broadcast = Broadcast::find_or_404(&state.db, id).await?;
    let def = type_def(&state.db, &broadcast.type_key).await?;
    let resolution = resolve(state, &broadcast, now).await?;
    let already: Vec<i64> = broadcast_results::messages_of(&state.db, id)
        .await?
        .into_iter()
        .filter_map(|m| m.party_id)
        .collect();
    let missing: Vec<&Recipient> = resolution
        .recipients
        .iter()
        .filter(|r| !already.contains(&r.party.0))
        .collect();

    let size = batching.size.max(1);
    for (n, batch) in missing.chunks(size).enumerate() {
        if batching.max_batches.is_some_and(|max| n >= max) {
            return Ok(Ran::Partial);
        }
        let mut tx = state.db.begin().await?;
        let Some(locked) = lock(&mut tx, id).await? else {
            return Ok(Ran::Skipped);
        };
        if locked.status != BroadcastStatus::Sending {
            return Ok(Ran::Skipped);
        }
        // A stopped broadcast queues nothing more (`NTF.RUL.03`).
        if locked.stopped_at.is_some() {
            break;
        }
        for recipient in batch {
            let queued = Message::query()
                .where_(Message::BROADCAST_ID.eq(id))
                .where_(Message::PARTY_ID.eq(recipient.party.0))
                .first(&mut tx)
                .await?;
            if queued.is_some() {
                continue;
            }
            let mut message = message_for(&def, &locked, recipient);
            message.insert(&mut tx).await?;
            message.chain_id = Some(message.id);
            // The brand and the way out, now that the id is known.
            opt_out::add_footer(state, &mut message)?;
            message.save(&mut tx).await?;
            state
                .queue
                .dispatch_in(
                    &mut tx,
                    SendMessage {
                        message_id: message.id,
                    },
                )
                .await?;
        }
        tx.commit().await?;
    }
    complete(state, id, resolution.excluded).await
}

/// `complete` with the counts, then `BroadcastCompleted`.
async fn complete(state: &AppState, id: i64, excluded: ExcludedCounts) -> Result<Ran> {
    let mut tx = state.db.begin().await?;
    let Some(locked) = lock(&mut tx, id).await? else {
        return Ok(Ran::Skipped);
    };
    if locked.status != BroadcastStatus::Sending {
        return Ok(Ran::Skipped);
    }
    let (count, results) = broadcast_results::compute(&mut tx, id).await?;
    let input = CompleteInput {
        recipients_resolved: count,
        excluded: excluded.clone(),
        results,
    };
    let outcome = locked
        .transition(COMPLETE)
        .by_system()
        .input(&json!(input))
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    state
        .emit(BroadcastCompleted {
            broadcast: locked.broadcast_id(),
            requester: locked.requester(),
            channel: locked.channel,
            recipients: count,
            excluded: excluded.0,
        })
        .await?;
    Ok(Ran::Completed)
}

/// Cancels broadcast `id` for `by`, the requester or a holder of
/// `notifications.marketing.send` (anyone else gets 422 from the machine's
/// guard). A request still waiting for approval is withdrawn through the
/// kernel by its requester, which cancels the broadcast and announces it; a
/// holder who is not the requester cancels it directly. Returns the
/// broadcast as it is after.
pub async fn cancel(
    state: &AppState,
    id: i64,
    by: &renox::AuthUser,
    reason: &str,
) -> Result<Broadcast> {
    let broadcast = Broadcast::find_or_404(&state.db, id).await?;
    if broadcast.status == BroadcastStatus::PendingApproval
        && broadcast.requested_by == by.id
        && let Some(request) = broadcast.approval_request_id
    {
        approvals::withdraw(state, request, by).await?;
        return Broadcast::find_or_404(&state.db, id).await;
    }
    let mut tx = state.db.begin().await?;
    let Some(locked) = lock(&mut tx, id).await? else {
        return Err(Error::Status(
            StatusCode::CONFLICT,
            "the broadcast is being changed".to_owned(),
        ));
    };
    let mut run = locked.transition(CANCEL).by(by);
    if !reason.trim().is_empty() {
        run = run.reason(reason);
    }
    let outcome = run
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let cancelled = Broadcast::find_or_404(&state.db, id).await?;
    announce_cancelled(state, &cancelled, BroadcastCancelReason::Cancelled).await?;
    Ok(cancelled)
}

/// Registers the [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            let now = clock(&state).now();
            run_due(&state, now).await.map(|_| ())
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_queues_100_messages_per_transaction_unless_told() {
        let batching = Batching::default();
        assert_eq!(batching.size, 100);
        assert_eq!(batching.max_batches, None);
        assert_eq!(TASK, "notify:broadcasts");
    }
}
