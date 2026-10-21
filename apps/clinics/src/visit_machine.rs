//! The visit machine.
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | `check_in` | — | `checked_in` | the booking's check-in (history row of the insert) |
//! | [`START`] | checked_in | in_progress | system (`BookingStarted`) |
//! | [`COMPLETE`] | checked_in, in_progress | completed | system (`BookingCompleted`) |
//! | [`CANCEL`] | checked_in, in_progress | cancelled | system (`BookingCancelled` after the check-in) |
//!
//! Decision: the spec has no machine of its own for the visit (§7.1 gives the
//! appointment's); the visit follows the booking, which is the one that
//! decides, so every transition is the system's, run by a listener on the
//! kernel's booking events ([`crate::visits`]). Cancelling before the
//! check-in opens no visit. Charge capture, the claim and the lots hang on
//! the visit later and add their own columns, not states.

use renox::db::DateTime;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::visit::{Visit, VisitStatus};

/// The history row of a visit coming to be.
pub const CHECK_IN: &str = "check_in";
/// The service started.
pub const START: &str = "start";
/// The service took place.
pub const COMPLETE: &str = "complete";
/// The booking was cancelled after the check-in.
pub const CANCEL: &str = "cancel";

/// What a run carries to the effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitInput {
    /// When it happened, by the booking's clock.
    #[serde(default)]
    pub at: Option<DateTime>,
    /// The booking's cancellation reason code (`cancel`).
    #[serde(default)]
    pub reason: Option<String>,
}

fn input_of(ctx: &TransitionCtx) -> VisitInput {
    ctx.input_as::<VisitInput>().unwrap_or_default()
}

impl StateMachine for Visit {
    type State = VisitStatus;
    const COLUMN: renox::db::Col<Visit, VisitStatus> = Visit::STATUS;

    fn machine() -> Machine<Self> {
        use VisitStatus::{Cancelled, CheckedIn, Completed, InProgress};
        Machine::new()
            .initial([CheckedIn])
            .state(CheckedIn)
            .tone("info")
            .state(InProgress)
            .tone("info")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(start())
            .transition(complete())
            .transition(cancel())
    }
}

/// `start`: the patient is with the practitioner.
fn start() -> Transition<Visit> {
    Transition::new(START)
        .label("Start")
        .system()
        .from([VisitStatus::CheckedIn])
        .to(VisitStatus::InProgress)
        .effect(|v: &Visit, ctx, tx| {
            let id = v.id;
            let at = input_of(ctx).at.unwrap_or_else(renox::db::now);
            Box::pin(async move {
                crate::db::set_visit(&mut *tx, id, &[("started_at", &Some(at))]).await
            })
        })
}

/// `complete`: the visit is over.
fn complete() -> Transition<Visit> {
    Transition::new(COMPLETE)
        .label("Complete")
        .system()
        .from([VisitStatus::CheckedIn, VisitStatus::InProgress])
        .to(VisitStatus::Completed)
        .effect(|v: &Visit, ctx, tx| {
            let id = v.id;
            let at = input_of(ctx).at.unwrap_or_else(renox::db::now);
            Box::pin(async move {
                crate::db::set_visit(&mut *tx, id, &[("ended_at", &Some(at))]).await
            })
        })
}

/// `cancel`: the visit stopped after the check-in; the reason code is the
/// booking's.
fn cancel() -> Transition<Visit> {
    Transition::new(CANCEL)
        .label("Cancel")
        .system()
        .from([VisitStatus::CheckedIn, VisitStatus::InProgress])
        .to(VisitStatus::Cancelled)
        .effect(|v: &Visit, ctx, tx| {
            let id = v.id;
            let input = input_of(ctx);
            let at = input.at.unwrap_or_else(renox::db::now);
            Box::pin(async move {
                crate::db::set_visit(
                    &mut *tx,
                    id,
                    &[("ended_at", &Some(at)), ("cancel_reason", &input.reason)],
                )
                .await
            })
        })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Visit>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = Visit::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("checked_in", Some("info"), false),
                ("in_progress", Some("info"), false),
                ("completed", Some("success"), true),
                ("cancelled", Some("neutral"), true),
            ]
        );
        let expected = [
            (START, vec!["checked_in"], "in_progress"),
            (COMPLETE, vec!["checked_in", "in_progress"], "completed"),
            (CANCEL, vec!["checked_in", "in_progress"], "cancelled"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(transition.is_system(), "{name}");
        }
    }
}
