//! How the messages of a broadcast ended up (wl-notify §3 *Broadcast*
//! results, #61 A3): counted from the messages, never stored apart from the
//! snapshot `complete` takes.

use renox::prelude::*;

use crate::broadcast::{BroadcastResults, ExcludedCounts};
use crate::broadcast_recipients::OPTED_OUT;
use crate::message::{FREQUENCY_CAP, Message, MessageStatus};
use crate::stale::{BROADCAST_STOPPED, EXPIRED};

/// The reasons a message gets after the broadcast completed (at send time or
/// by `NTF.RUL.03`), which join the excluded counts.
const SEND_TIME_REASONS: [&str; 3] = [FREQUENCY_CAP, EXPIRED, BROADCAST_STOPPED];

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

/// The members left out at resolve time (`stored`) and the messages
/// `suppressed` at send time for the frequency cap, counted from the messages
/// by reason (#62 A9). Those messages only get their reason when they are
/// sent, after the broadcast completed, so they are not in the stored counts.
pub fn excluded_with_cap<'a>(
    stored: &ExcludedCounts,
    messages: impl IntoIterator<Item = (MessageStatus, Option<&'a str>)>,
) -> ExcludedCounts {
    let mut counts = stored.clone();
    let mut later: Vec<&str> = Vec::new();
    for (status, reason) in messages {
        if status == MessageStatus::Suppressed
            && let Some(reason) = reason
            && SEND_TIME_REASONS.contains(&reason)
        {
            later.push(reason);
        }
    }
    for reason in &SEND_TIME_REASONS {
        let n = later.iter().filter(|r| *r == reason).count() as i64;
        if n > 0 {
            counts.0.insert((*reason).to_owned(), n);
        }
    }
    counts
}

/// The excluded counts of broadcast `id` now: see [`excluded_with_cap`].
pub async fn excluded<'c>(
    db: impl renox::db::Executor<'c>,
    id: i64,
    stored: &ExcludedCounts,
) -> Result<ExcludedCounts> {
    let messages = messages_of(db, id).await?;
    Ok(excluded_with_cap(
        stored,
        messages
            .iter()
            .map(|m| (m.status, m.suppress_reason.as_deref())),
    ))
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

    #[test]
    fn stale_and_stopped_messages_join_the_excluded_counts() {
        use MessageStatus::*;
        let counts = excluded_with_cap(
            &ExcludedCounts::default(),
            [
                (Suppressed, Some("expired")),
                (Suppressed, Some("expired")),
                (Suppressed, Some("broadcast_stopped")),
                (Sent, None),
            ],
        );
        assert_eq!(counts.of("expired"), 2);
        assert_eq!(counts.of("broadcast_stopped"), 1);
        assert_eq!(counts.total(), 3);
    }

    #[test]
    fn messages_suppressed_by_the_cap_join_the_excluded_counts() {
        use MessageStatus::*;
        let mut stored = ExcludedCounts::default();
        stored.add("no_consent");
        let counts = excluded_with_cap(
            &stored,
            [
                (Sent, None),
                (Suppressed, Some("frequency_cap")),
                (Suppressed, Some("frequency_cap")),
                (Suppressed, Some("opted_out")),
            ],
        );
        assert_eq!(counts.of("frequency_cap"), 2);
        assert_eq!(counts.of("no_consent"), 1);
        assert_eq!(counts.total(), 3);
        let none = excluded_with_cap(&stored, [(Sent, None)]);
        assert_eq!(none, stored);
    }
}
