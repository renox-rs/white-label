//! How the messages of a broadcast ended up (wl-notify §3 *Broadcast*
//! results, #61 A3): counted from the messages, never stored apart from the
//! snapshot `complete` takes.

use renox::prelude::*;

use crate::broadcast::BroadcastResults;
use crate::broadcast_recipients::OPTED_OUT;
use crate::message::{Message, MessageStatus};

/// The results for messages with the given statuses (and suppress reasons).
pub fn tally<'a>(
    messages: impl IntoIterator<Item = (MessageStatus, Option<&'a str>)>,
) -> BroadcastResults {
    let mut results = BroadcastResults::default();
    for (status, reason) in messages {
        match status {
            MessageStatus::Sent => results.sent += 1,
            MessageStatus::Delivered | MessageStatus::Read => {
                results.sent += 1;
                results.delivered += 1;
            }
            MessageStatus::Failed | MessageStatus::Abandoned => results.failed += 1,
            MessageStatus::Suppressed if reason == Some(OPTED_OUT) => results.opted_out += 1,
            MessageStatus::Queued | MessageStatus::Deferred | MessageStatus::Suppressed => {}
        }
    }
    results
}

/// The messages of broadcast `id`, read through `db`.
pub async fn messages_of<'c>(db: impl renox::db::Executor<'c>, id: i64) -> Result<Vec<Message>> {
    Message::query()
        .where_(Message::BROADCAST_ID.eq(id))
        .order_by(Message::ID)
        .get(db)
        .await
}

/// The number of messages of broadcast `id` now, and their results.
pub async fn compute<'c>(
    db: impl renox::db::Executor<'c>,
    id: i64,
) -> Result<(i64, BroadcastResults)> {
    let messages = messages_of(db, id).await?;
    let results = tally(
        messages
            .iter()
            .map(|m| (m.status, m.suppress_reason.as_deref())),
    );
    Ok((messages.len() as i64, results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_results_count_each_status_once() {
        use MessageStatus::*;
        let results = tally([
            (Queued, None),
            (Sent, None),
            (Delivered, None),
            (Read, None),
            (Failed, None),
            (Abandoned, None),
            (Suppressed, Some("opted_out")),
            (Suppressed, Some("no_consent")),
        ]);
        assert_eq!(
            results,
            BroadcastResults {
                sent: 3,
                delivered: 2,
                failed: 2,
                opted_out: 1
            }
        );
    }
}
