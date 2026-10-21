//! Sending a message (wl-notify §4.1 `send`, `fail`, `retry`; #55 A7, A8).
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
//!   [`retry_backoff`] (1 minute, then 5 minutes). Any other error is final
//!   on the channel: in the same transaction the message falls back to the
//!   next allowed channel, or gives up and queues `NTF.RUL.01`
//!   ([`crate::give_up`]); `MessageGaveUp` follows the commit.

use std::time::Duration;

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::notify::{Category, MessageFailed, NotifyChannel, Urgency};

use crate::broadcast_recipients;
use crate::cap;
use crate::channels::{self, Outgoing, SendError, SendErrorKind};
use crate::give_up::{self, Applied};
use crate::machine::{DEFER, FAIL, RETRY, SEND, SUPPRESS};
use crate::message::{FREQUENCY_CAP, INVALID_CONTACT_POINT, Message, MessageStatus, RecipientKind};
use crate::send_window::SendWindow;
use crate::suppressions;
use crate::window;

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
    /// A send window or the quiet hours hold it: `deferred` until the time.
    Deferred(DateTime),
    /// It failed and is queued again for a send after the wait.
    Retrying(Duration),
    /// It failed for good on its channel and fell back: the new message's id.
    FellBack(i64),
    /// It failed for good and no channel is left: `abandoned` by `give_up`.
    GaveUp,
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

/// The `AD-...` sender ID a marketing SMS of a broadcast goes out with.
fn sender_id(state: &AppState, message: &Message) -> Option<String> {
    if message.channel != wl_kernel::notify::NotifyChannel::Sms || message.broadcast_id.is_none() {
        return None;
    }
    crate::broadcast::settings(state)
        .valid_sms_sender_id()
        .map(str::to_owned)
}

/// Sends message `id` once, if it is `queued`.
///
/// "Now" is the module's clock ([`crate::clock`]).
pub async fn send(state: &AppState, id: i64) -> Result<SendOutcome> {
    send_at(state, id, crate::clock::clock(state).now()).await
}

/// Sends message `id` once, if it is `queued`, as of `now`: the instant the
/// send window and the quiet hours are checked against and `sent_at` is set to.
pub async fn send_at(state: &AppState, id: i64, now: DateTime) -> Result<SendOutcome> {
    let Some(message) = Message::find(&state.db, id).await? else {
        return Ok(SendOutcome::Skipped);
    };
    if message.status != MessageStatus::Queued {
        return Ok(SendOutcome::Skipped);
    }
    if let Some(outcome) = invalid_point_guard(state, &message).await? {
        outcome.emit(state).await?;
        return Ok(SendOutcome::Skipped);
    }
    if let Some(outcome) = consent_guard(state, &message).await? {
        outcome.emit(state).await?;
        return Ok(SendOutcome::Skipped);
    }
    if let Some(outcome) = crate::stale::send_guard(state, &message, now).await? {
        outcome.emit(state).await?;
        return Ok(SendOutcome::Skipped);
    }
    if let Some(until) = hold_until(state, &message, now).await? {
        defer(state, &message, until).await?;
        return Ok(SendOutcome::Deferred(until));
    }
    // A capped message holds its party's lock from the count through the
    // provider call and the `sent` commit, so a concurrent send counts it.
    let party_lock = match (cap::applies(state, &message).await?, message.party_id) {
        (true, Some(party)) => Some(cap::lock(party).await),
        _ => None,
    };
    if party_lock.is_some() {
        // Another send may have changed the message while this one waited.
        let current = Message::find(&state.db, id).await?;
        if current.is_none_or(|m| m.status != MessageStatus::Queued) {
            return Ok(SendOutcome::Skipped);
        }
        if cap::reached(state, &message, now).await? {
            let outcome = cap_suppress(state, &message).await?;
            outcome.emit(state).await?;
            return Ok(SendOutcome::Skipped);
        }
    }
    let provider = channels::providers(state)
        .for_channel(message.channel)
        .filter(|p| p.available(state));
    let (name, result) = match (provider, message.address.as_deref()) {
        (Some(provider), Some(address)) => {
            let outgoing = Outgoing::of(&message, address)
                .with_sender_id(sender_id(state, &message).as_deref());
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
    report_health(state, message.channel, name, &result).await?;
    match result {
        Ok(provider_message_id) => sent(state, message, name, provider_message_id, now).await,
        Err(error) => {
            // A failed attempt does not count: the lock is not held for it.
            drop(party_lock);
            failed(state, message, name, error).await
        }
    }
}

/// Reports how the provider of the channel answered to the integration
/// health ([`wl_kernel::integrations::EMAIL`], `SMS` or `WHATSAPP`), when the
/// app registered the connection: an accepted send is a success and a
/// transient failure a failure. A permanent or hard error (a wrong address)
/// says nothing about the connection, nor does a message no provider tried.
async fn report_health(
    state: &AppState,
    channel: NotifyChannel,
    provider: Option<&str>,
    result: &std::result::Result<String, SendError>,
) -> Result {
    if provider.is_none() {
        return Ok(());
    }
    let outcome = match result {
        Ok(_) => Ok(()),
        Err(error) if error.kind == SendErrorKind::Transient => Err(error.message.as_str()),
        Err(_) => return Ok(()),
    };
    let key = match channel {
        NotifyChannel::Email => wl_kernel::integrations::EMAIL,
        NotifyChannel::Sms => wl_kernel::integrations::SMS,
        NotifyChannel::Whatsapp => wl_kernel::integrations::WHATSAPP,
    };
    wl_kernel::integrations::report_outcome(state, key, outcome).await
}

/// The `send` guard "frequency cap" (§4.1, `NTF.RUL.02`): the message is
/// `suppressed` with `frequency_cap`.
async fn cap_suppress(state: &AppState, message: &Message) -> Result<Outcome<Message>> {
    let mut tx = state.db.begin().await?;
    let outcome = run(message, SUPPRESS, Some(FREQUENCY_CAP.to_owned()), &mut tx).await?;
    tx.commit().await?;
    Ok(outcome)
}

/// The `send` guard "contact point valid" (§4.1): a party message whose
/// contact point became invalid after it was queued is `suppressed` with
/// `invalid_contact_point` and not sent. Users, bare addresses and points the
/// directory does not know pass.
async fn invalid_point_guard(
    state: &AppState,
    message: &Message,
) -> Result<Option<Outcome<Message>>> {
    let (Some(party), Some(point)) = (message.party_id, message.contact_point_id) else {
        return Ok(None);
    };
    let contacts = wl_kernel::parties::directory(state)
        .contacts(&state.db, wl_kernel::PartyId(party))
        .await?;
    if !point_is_invalid(contacts.as_ref(), point) {
        return Ok(None);
    }
    let mut tx = state.db.begin().await?;
    let outcome = run(
        message,
        SUPPRESS,
        Some(INVALID_CONTACT_POINT.to_owned()),
        &mut tx,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(outcome))
}

/// The `send` guard "marketing consent" (§4.1, `NTF.ACL.01`): a marketing
/// message to a party whose consent is gone is `suppressed` with `no_consent`.
async fn consent_guard(state: &AppState, message: &Message) -> Result<Option<Outcome<Message>>> {
    if !broadcast_recipients::lacks_consent(state, message).await? {
        return Ok(None);
    }
    let mut tx = state.db.begin().await?;
    let outcome = run(
        message,
        SUPPRESS,
        Some(broadcast_recipients::NO_CONSENT.to_owned()),
        &mut tx,
    )
    .await?;
    tx.commit().await?;
    Ok(Some(outcome))
}

/// The `send` guard "send window and quiet hours" (§4.1, `NTF.RUL.02`): when
/// the message must wait, the time it is released. The category comes from the
/// message's type, as the message does not store it.
///
/// - `marketing` outside its channel's window waits for the next opening;
/// - any other category of normal urgency to an external recipient (not a
///   staff user) inside the quiet hours waits for their end. An `urgent` type
///   is never held.
async fn hold_until(
    state: &AppState,
    message: &Message,
    now: DateTime,
) -> Result<Option<DateTime>> {
    let def = crate::notifier::type_def(&state.db, &message.type_key).await?;
    if def.category == Category::Marketing {
        let window = SendWindow::for_channel(&state.db, message.channel).await?;
        return Ok((!window::in_window(window, now)).then(|| window::next_opening(window, now)));
    }
    let external = message.recipient_kind != RecipientKind::User;
    if def.urgency == Urgency::Normal && external && window::in_quiet_hours(now) {
        return Ok(Some(window::next_quiet_end(now)));
    }
    Ok(None)
}

/// `defer`: stores `scheduled_for`, then holds the message.
async fn defer(state: &AppState, message: &Message, until: DateTime) -> Result {
    let mut message = message.clone();
    let mut tx = state.db.begin().await?;
    message.scheduled_for = Some(until);
    message.save(&mut tx).await?;
    let outcome = run(&message, DEFER, None, &mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await
}

/// Whether `contacts` knows contact point `point` and it is not valid.
pub fn point_is_invalid(contacts: Option<&wl_kernel::notify::PartyContacts>, point: i64) -> bool {
    contacts.is_some_and(|c| c.points.iter().any(|p| p.id == point && !p.valid))
}

/// `send`: stores who sent it and when.
async fn sent(
    state: &AppState,
    mut message: Message,
    provider: Option<&str>,
    provider_message_id: String,
    now: DateTime,
) -> Result<SendOutcome> {
    let mut tx = state.db.begin().await?;
    // The accepted send is an attempt too.
    message.attempts += 1;
    message.provider = provider.map(str::to_owned);
    message.provider_message_id = Some(provider_message_id);
    message.sent_at = Some(now);
    message.save(&mut tx).await?;
    let outcome = run(&message, SEND, None, &mut tx).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    Ok(SendOutcome::Sent)
}

/// `fail`, then `retry` while a transient error has attempts left, else
/// `fall_back` or `give_up`.
async fn failed(
    state: &AppState,
    mut message: Message,
    provider: Option<&str>,
    error: SendError,
) -> Result<SendOutcome> {
    let next = after_failure(error.kind, message.attempts + 1);
    // Read before the transaction: the next channel of the chain, if any.
    let plan = match next {
        AfterFailure::Retry(_) => None,
        AfterFailure::Final => Some(give_up::plan(state, &message).await?),
    };
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
    let failed = Message::find(&mut tx, message.id)
        .await?
        .ok_or(renox::Error::NotFound)?;
    let applied = match &plan {
        None => {
            outcomes.push(run(&failed, RETRY, None, &mut tx).await?);
            None
        }
        Some(plan) => {
            let (outcome, applied) = give_up::apply_in(state, &mut tx, &failed, plan).await?;
            outcomes.push(outcome);
            Some(applied)
        }
    };
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
    match applied {
        None => {
            let wait = retry_backoff(message.attempts);
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
        Some(Applied::FellBack(id)) => Ok(SendOutcome::FellBack(id)),
        Some(Applied::GaveUp) => {
            give_up::emit_gave_up(state, message.id).await?;
            Ok(SendOutcome::GaveUp)
        }
    }
}

/// Runs the system transition `name` on `message` inside `tx`.
pub(crate) async fn run(
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
    use wl_kernel::PartyId;
    use wl_kernel::notify::{ContactKind, PartyContactPoint, PartyContacts};

    use crate::channels::HardError;

    fn contacts(valid: bool) -> PartyContacts {
        PartyContacts {
            party: PartyId(3),
            language: "en".into(),
            preferred_channel: None,
            points: vec![PartyContactPoint {
                id: 11,
                kind: ContactKind::Email,
                address: "ali@bluedunes.ae".into(),
                primary: true,
                verified: false,
                valid,
            }],
        }
    }

    #[test]
    fn the_guard_stops_only_a_known_invalid_point() {
        assert!(point_is_invalid(Some(&contacts(false)), 11));
        assert!(!point_is_invalid(Some(&contacts(true)), 11));
        assert!(!point_is_invalid(Some(&contacts(false)), 12));
        assert!(!point_is_invalid(None, 11));
    }

    #[test]
    fn the_guard_reason_is_stable() {
        assert_eq!(INVALID_CONTACT_POINT, "invalid_contact_point");
    }

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
