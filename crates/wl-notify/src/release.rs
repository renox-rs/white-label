//! Releasing deferred messages (wl-notify §4.1 `release`, #62 A5).
//!
//! A message held by a send window or the quiet hours ([`crate::send`]) is
//! `deferred` with its `scheduled_for`. The scheduled task [`TASK`] runs
//! [`run_due`] every minute: each deferred message whose time has come runs
//! `release` (`deferred -> queued`) and its [`SendMessage`] job is queued in
//! the same transaction. The send then checks everything again, so a message
//! released into a window that closed meanwhile is deferred again.

use renox::db::DateTime;
use renox::prelude::*;

use crate::machine::RELEASE;
use crate::message::{Message, MessageStatus};
use crate::send::{SendMessage, run};

/// The scheduled task that runs [`run_due`] every minute.
pub const TASK: &str = "notify:release";

/// Releases every deferred message scheduled for `now` or earlier and queues
/// its send. Returns how many were released. A message another run released
/// first is skipped, so a message is released once.
pub async fn run_due(state: &AppState, now: DateTime) -> Result<usize> {
    let due = Message::query()
        .where_(Message::STATUS.eq(MessageStatus::Deferred))
        .where_(Message::SCHEDULED_FOR.lte(now))
        .order_by(Message::ID)
        .get(&state.db)
        .await?;
    let mut released = 0;
    for message in due {
        let mut tx = state.db.begin().await?;
        // Read again inside the transaction: another run may have released it.
        let Some(current) = Message::find(&mut tx, message.id).await? else {
            continue;
        };
        if current.status != MessageStatus::Deferred {
            continue;
        }
        let outcome = run(&current, RELEASE, None, &mut tx).await?;
        state
            .queue
            .dispatch_in(
                &mut tx,
                SendMessage {
                    message_id: current.id,
                },
            )
            .await?;
        tx.commit().await?;
        outcome.emit(state).await?;
        released += 1;
    }
    Ok(released)
}

/// Registers the [`TASK`].
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .every_minute(TASK, |state: AppState| async move {
            let now = crate::clock::clock(&state).now();
            run_due(&state, now).await.map(|_| ())
        });
}
