//! The Message state machine (wl-notify §4.1).
//!
//! ```text
//! [*] --> queued --defer--> deferred --release--> queued
//!         queued --send--> sent --deliver--> delivered --mark_read--> read
//!         queued | deferred --suppress--> suppressed
//!         queued | sent --fail--> failed --retry--> queued
//!         failed --fall_back | give_up--> abandoned
//! ```
//!
//! Every state and transition is declared here, all run by the system, and
//! a new message is inserted only as `queued`. `Message` is
//! `#[model(state_machine)]`, so `save` refuses a changed status. `suppress`
//! records its reason and, after the commit, emits the kernel's
//! [`MessageSuppressed`] ([`register`]). `send`, `fail` and `retry` are
//! run by the send job in [`crate::send`], which stores the provider's id,
//! the attempts and the error with them; `fall_back` and `give_up` come
//! with #55 part 6; `defer`, `release`, `deliver` and `mark_read` have no
//! caller yet.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, Transitioned};
use wl_kernel::notify::MessageSuppressed;

use crate::message::{Message, MessageStatus};

/// `queued → deferred`: held for quiet hours or a send window.
pub const DEFER: &str = "defer";
/// `deferred → queued`: the scheduled time is reached.
pub const RELEASE: &str = "release";
/// `queued → sent`: the provider accepted it.
pub const SEND: &str = "send";
/// `queued | deferred → suppressed`: not sent by rule.
pub const SUPPRESS: &str = "suppress";
/// `sent → delivered`: a delivery receipt.
pub const DELIVER: &str = "deliver";
/// `sent | delivered → read`: a read receipt.
pub const MARK_READ: &str = "mark_read";
/// `queued | sent → failed`: a provider error or a failed receipt.
pub const FAIL: &str = "fail";
/// `failed → queued`: another attempt on the same channel.
pub const RETRY: &str = "retry";
/// `failed → abandoned`: a new message goes out on the next channel.
pub const FALL_BACK: &str = "fall_back";
/// `failed → abandoned`: no retry or fallback is left.
pub const GIVE_UP: &str = "give_up";

impl StateMachine for Message {
    type State = MessageStatus;
    const COLUMN: renox::db::Col<Message, MessageStatus> = Message::STATUS;

    fn machine() -> Machine<Self> {
        use MessageStatus::{
            Abandoned, Deferred, Delivered, Failed, Queued, Read, Sent, Suppressed,
        };
        Machine::new()
            .initial([Queued])
            .state(Queued)
            .tone("neutral")
            .state(Deferred)
            .tone("neutral")
            .state(Sent)
            .tone("info")
            .state(Delivered)
            .tone("success")
            .state(Read)
            .tone("success")
            .terminal()
            .state(Failed)
            .tone("error")
            .state(Suppressed)
            .tone("neutral")
            .terminal()
            .state(Abandoned)
            .tone("error")
            .terminal()
            .transition(system(DEFER, "Defer", [Queued], Deferred))
            .transition(system(RELEASE, "Release", [Deferred], Queued))
            .transition(system(SEND, "Send", [Queued], Sent))
            .transition(
                system(SUPPRESS, "Suppress", [Queued, Deferred], Suppressed).effect(
                    |message: &Message, ctx, tx| {
                        let id = message.id;
                        let reason = ctx.reason().map(str::to_owned);
                        Box::pin(async move {
                            // Not the status column, so outside the machine's guard.
                            renox::db::sql(format!(
                                "UPDATE {} SET suppress_reason = ?, updated_at = ? WHERE id = ?",
                                Message::TABLE
                            ))
                            .bind(reason)
                            .bind(renox::db::now())
                            .bind(id)
                            .execute(&mut *tx)
                            .await?;
                            Ok(())
                        })
                    },
                ),
            )
            .transition(system(DELIVER, "Deliver", [Sent], Delivered))
            .transition(system(MARK_READ, "Mark read", [Sent, Delivered], Read))
            .transition(system(FAIL, "Fail", [Sent, Queued], Failed))
            .transition(system(RETRY, "Retry", [Failed], Queued))
            .transition(system(FALL_BACK, "Fall back", [Failed], Abandoned))
            .transition(system(GIVE_UP, "Give up", [Failed], Abandoned))
    }
}

/// A system transition `name` from `from` to `to`.
fn system<const N: usize>(
    name: &str,
    label: &str,
    from: [MessageStatus; N],
    to: MessageStatus,
) -> Transition<Message> {
    Transition::new(name)
        .label(label)
        .from(from)
        .to(to)
        .system()
}

/// Emits [`MessageSuppressed`] after a `suppress` commits.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Message>, state: AppState| async move {
        if event.transition != SUPPRESS {
            return Ok(());
        }
        let Some(message) = Message::find(&state.db, event.key).await? else {
            return Ok(());
        };
        state
            .emit(MessageSuppressed {
                message: message.message_id(),
                type_key: message.type_key.clone(),
                subject: message.subject(),
                recipient: message.recipient(),
                reason: event.reason.unwrap_or_default(),
            })
            .await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared_as_system() {
        let machine = Message::machine();
        let table = [
            (DEFER, &["queued"][..], "deferred"),
            (RELEASE, &["deferred"], "queued"),
            (SEND, &["queued"], "sent"),
            (SUPPRESS, &["queued", "deferred"], "suppressed"),
            (DELIVER, &["sent"], "delivered"),
            (MARK_READ, &["sent", "delivered"], "read"),
            (FAIL, &["sent", "queued"], "failed"),
            (RETRY, &["failed"], "queued"),
            (FALL_BACK, &["failed"], "abandoned"),
            (GIVE_UP, &["failed"], "abandoned"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(transition.is_system(), "{name}");
        }
    }

    #[test]
    fn a_message_starts_queued_and_read_suppressed_abandoned_are_terminal() {
        let machine = Message::machine();
        assert_eq!(machine.initial_states(), ["queued"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["read", "suppressed", "abandoned"]);
        assert_eq!(machine.states().len(), MessageStatus::ALL.len());
    }
}
