//! Stopped recipients (wl-notify §6 `stop_messages_to`, wl-hr `record_death`):
//! every message to a stopped user or party is cancelled, the ones waiting now
//! and the ones that would be queued later.
//!
//! - **Waiting now.** A `queued` or `deferred` message whose recipient is the
//!   user or party (a rule message is matched by the person it chose) is
//!   `suppress`ed with the reason given; one already sent stays.
//! - **Later.** The recipient is kept in `wl_notify_stopped_recipients`, and
//!   [`is_stopped`] makes [`crate::notifier::queue`] suppress a new message at
//!   once (the same transition, the same `MessageSuppressed`). A bare address
//!   or a rule over other people's contacts is not stopped: only the user and
//!   the party are.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::NotifyRecipient;

use crate::machine::SUPPRESS;
use crate::message::{Message, MessageStatus, RecipientKind};

/// Why a message to a stopped recipient is `suppressed` when it is queued.
pub const STOPPED: &str = "recipient_stopped";

/// A user or party that gets no message.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_notify_stopped_recipients",
    unique(recipient_kind, recipient_id)
)]
pub struct StoppedRecipient {
    /// The primary key.
    pub id: i64,
    /// `user` or `party`.
    pub recipient_kind: String,
    /// The user's or party's id.
    pub recipient_id: i64,
    /// Why.
    pub reason: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The kind and id a stopped recipient is kept under, or `None` for a
/// recipient that cannot be stopped (an address, a rule).
fn key(recipient: &NotifyRecipient) -> Option<(&'static str, i64)> {
    match recipient {
        NotifyRecipient::User { id } => Some(("user", *id)),
        NotifyRecipient::Party { id } => Some(("party", id.0)),
        _ => None,
    }
}

/// Whether `recipient` is stopped.
pub async fn is_stopped(db: &Db, recipient: &NotifyRecipient) -> Result<bool> {
    let Some((kind, id)) = key(recipient) else {
        return Ok(false);
    };
    StoppedRecipient::query()
        .where_(StoppedRecipient::RECIPIENT_KIND.eq(kind.to_owned()))
        .where_(StoppedRecipient::RECIPIENT_ID.eq(id))
        .exists(db)
        .await
}

/// Stops `recipients` for `reason`: the waiting messages are cancelled and
/// later ones are suppressed. Returns how many waiting messages were
/// cancelled.
pub async fn stop(state: &AppState, recipients: &[NotifyRecipient], reason: &str) -> Result<usize> {
    let mut tx = state.db.begin().await?;
    let mut outcomes = Vec::new();
    for recipient in recipients {
        let Some((kind, id)) = key(recipient) else {
            continue;
        };
        add_in(&mut tx, kind, id, reason).await?;
        for message in waiting_in(&mut tx, kind, id).await? {
            let outcome = message
                .transition(SUPPRESS)
                .by_system()
                .reason(reason)
                .run_in(&mut tx)
                .await
                .map_err(TransitionError::into_error)?;
            outcomes.push(outcome);
        }
    }
    tx.commit().await?;
    let count = outcomes.len();
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    Ok(count)
}

async fn add_in(tx: &mut Transaction, kind: &str, id: i64, reason: &str) -> Result {
    let known = StoppedRecipient::query()
        .where_(StoppedRecipient::RECIPIENT_KIND.eq(kind.to_owned()))
        .where_(StoppedRecipient::RECIPIENT_ID.eq(id))
        .exists(&mut *tx)
        .await?;
    if !known {
        StoppedRecipient::create(
            &mut *tx,
            StoppedRecipient {
                recipient_kind: kind.to_owned(),
                recipient_id: id,
                reason: reason.to_owned(),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// The `queued` and `deferred` messages for the user or party.
async fn waiting_in(tx: &mut Transaction, kind: &str, id: i64) -> Result<Vec<Message>> {
    let query = Message::query()
        .where_(Message::STATUS.is_in([MessageStatus::Queued, MessageStatus::Deferred]));
    let query = if kind == "user" {
        query
            .where_(Message::RECIPIENT_KIND.eq(RecipientKind::User))
            .where_(Message::USER_ID.eq(id))
    } else {
        // A party message, or a rule message that chose this person.
        query.where_(Message::PARTY_ID.eq(id))
    };
    query.order_by(Message::ID).get(&mut *tx).await
}
