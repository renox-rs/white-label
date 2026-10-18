//! Sending a message (wl-notify §4.1 `send`, `fail`, `retry`; #55 A7).
//!
//! The [`SendMessage`] job hands a `queued` message to its channel's
//! provider ([`crate::channels`]). The message row is the truth about
//! attempts and errors, so the module drives the retries itself instead of
//! Renox job retries (those only cover the database work around a send):
//!
//! - accepted: `send` stores the provider, its message id and `sent_at`;
//! - refused: `fail` counts the attempt and stores the error code, text and
//!   kind; a hard error puts the address on the suppression list. After
//!   the commit, `MessageFailed` is emitted.
//! - a transient error with attempts left ([`MAX_SEND_ATTEMPTS`]) runs
//!   `retry` in the same transaction and queues the next send after
//!   [`retry_backoff`] (1 minute, then 5 minutes). Any other error leaves
//!   the message `failed`, for the fallback or give-up of #55 part 6.

use std::time::Duration;

use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::MessageFailed;

use crate::channels::{self, Outgoing, SendError, SendErrorKind};
use crate::machine::{FAIL, RETRY, SEND};
use crate::message::{Message, MessageStatus};
use crate::suppressions;

/// Sends on one channel before the message is given up on that channel
/// (wl-notify §7 "3 attempts with backoff; then fall back").
pub const MAX_SEND_ATTEMPTS: i64 = 3;

/// The wait before the send after failed attempt `attempts` (1-based):
/// 1 minute after the first, 5 minutes after the later ones.
pub fn retry_backoff(attempts: i64) -> Duration {
    if attempts <= 1 {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(5 * 60)
    }
}

/// What follows a `fail` with an error of `kind`, the message having had
/// `attempts` sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterFailure {
    /// `retry`, and send again after the wait.
    Retry(Duration),
    /// No retry on this channel: fall back or give up.
    Final,
}

/// The step after a failure: retry a transient error while attempts are
/// left; anything else is final.
pub fn after_failure(kind: SendErrorKind, attempts: i64) -> AfterFailure {
    if kind == SendErrorKind::Transient && attempts < MAX_SEND_ATTEMPTS {
        AfterFailure::Retry(retry_backoff(attempts))
    } else {
        AfterFailure::Final
    }
}

/// What one run of [`send`] came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The message is gone or not `queued` (sent already, suppressed):
    /// nothing done.
    Skipped,
    /// The provider accepted it.
    Sent,
    /// It failed and is queued again for a send after the wait.
    Retrying(Duration),
    /// It failed and stays `failed`.
    Failed,
}

/// The job that sends one queued message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessage {
    /// The message's id.
    pub message_id: i64,
}

impl Job for SendMessage {
    const NAME: &'static str = "wl-notify.send-message";

    async fn handle(self, ctx: JobContext) -> Result {
        send(&ctx.state, self.message_id).await.map(|_| ())
    }
}

/// Sends message `id` once, if it is `queued`.
pub async fn send(state: &AppState, id: i64) -> Result<SendOutcome> {
    let Some(message) = Message::find(&state.db, id).await? else {
        return Ok(SendOutcome::Skipped);
    };
    if message.status != MessageStatus::Queued {
        return Ok(SendOutcome::Skipped);
    }
    let provider = channels::providers(state)
        .for_channel(message.channel)
        .filter(|p| p.available(state));
    let (name, result) = match (provider, message.address.as_deref()) {
        (Some(provider), Some(address)) => {
            let outgoing = Outgoing::of(&message, address);
            (Some(provider.name()), provider.send(state, &outgoing).await)
        }
        (None, _) => (
            None,
            Err(SendError::permanent(
                "no_provider",
                format!("no provider can send on {}", message.channel),
            )),
        ),
        (Some(provider), None) => (
            Some(provider.name()),
            Err(SendError::permanent(
                "no_address",
                "the message has no address",
            )),
        ),
    };
    match result {
        Ok(provider_message_id) => sent(state, message, name, provider_message_id).await,
        Err(error) => failed(state, message, name, error).await,
    }
}

/// `send`: stores who sent it and when.
async fn sent(
    state: &AppState,
    mut message: Message,
    provider: Option<&str>,
    provider_message_id: String,
) -> Result<SendOutcome> {
    let mut tx = state.db.begin().await?;
    message.provider = provider.map(str::to_owned);
    message.provider_message_id = Some(provider_message_id);
    message.sent_at = Some(renox::db::now());
    message.save(&mut tx).await?;
    let outcome = run(&message, SEND, None, &mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(SendOutcome::Sent)
}

/// `fail`, then `retry` while a transient error has attempts left.
async fn failed(
    state: &AppState,
    mut message: Message,
    provider: Option<&str>,
    error: SendError,
) -> Result<SendOutcome> {
    let mut tx = state.db.begin().await?;
    message.provider = provider.map(str::to_owned);
    message.attempts += 1;
    message.last_error_code = error.code.clone();
    message.last_error = Some(error.message.clone());
    message.error_kind = Some(error.kind.as_str().to_owned());
    message.save(&mut tx).await?;
    let mut outcomes = vec![run(&message, FAIL, Some(error.to_string()), &mut tx).await?];
    if let (Some(reason), Some(address)) = (error.kind.suppression(), message.address.as_deref()) {
        let source = format!("message:{}", message.id);
        suppressions::add(&mut tx, message.channel, address, reason, Some(&source)).await?;
    }
    let next = after_failure(error.kind, message.attempts);
    if let AfterFailure::Retry(_) = next {
        let failed = Message::find(&mut tx, message.id)
            .await?
            .ok_or(renox::Error::NotFound)?;
        outcomes.push(run(&failed, RETRY, None, &mut tx).await?);
    }
    tx.commit().await?;

    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    state
        .emit(MessageFailed {
            message: message.message_id(),
            type_key: message.type_key.clone(),
            subject: message.subject(),
            channel: message.channel,
            error_code: error.code.clone(),
            reason: error.message.clone(),
            is_final: next == AfterFailure::Final,
        })
        .await?;
    match next {
        AfterFailure::Retry(wait) => {
            state
                .queue
                .dispatch_after(
                    SendMessage {
                        message_id: message.id,
                    },
                    wait,
                )
                .await?;
            Ok(SendOutcome::Retrying(wait))
        }
        AfterFailure::Final => Ok(SendOutcome::Failed),
    }
}

/// Runs the system transition `name` on `message` inside `tx`.
async fn run(
    message: &Message,
    name: &str,
    reason: Option<String>,
    tx: &mut renox::db::Transaction,
) -> Result<Outcome<Message>> {
    let mut builder = message.transition(name).by_system();
    if let Some(reason) = reason {
        builder = builder.reason(reason);
    }
    builder
        .run_in(tx)
        .await
        .map_err(TransitionError::into_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::HardError;

    #[test]
    fn the_backoff_is_one_then_five_minutes() {
        assert_eq!(retry_backoff(1), Duration::from_secs(60));
        assert_eq!(retry_backoff(2), Duration::from_secs(300));
    }

    #[test]
    fn a_transient_error_is_retried_twice_then_final() {
        let transient = SendErrorKind::Transient;
        assert_eq!(
            after_failure(transient, 1),
            AfterFailure::Retry(Duration::from_secs(60))
        );
        assert_eq!(
            after_failure(transient, 2),
            AfterFailure::Retry(Duration::from_secs(300))
        );
        assert_eq!(after_failure(transient, 3), AfterFailure::Final);
    }

    #[test]
    fn permanent_and_hard_errors_are_final_at_once() {
        assert_eq!(
            after_failure(SendErrorKind::Permanent, 1),
            AfterFailure::Final
        );
        assert_eq!(
            after_failure(SendErrorKind::Hard(HardError::HardBounce), 1),
            AfterFailure::Final
        );
    }
}
