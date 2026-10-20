//! The Run state machine (wl-delivery §4.2).
//!
//! ```text
//! [*] --> planned --dispatch--> dispatched
//! ```
//!
//! `cancelled` is declared for the run `cancel` of a later issue. `dispatched` goes on to `returned_to_depot` and `closed` (BL-DLV-08,
//! BL-DLV-11); the states are declared now so the diagram is whole. The
//! driver and vehicle document guard of `dispatch` runs before the
//! transition ([`crate::doc_guard`]), because a Renox guard gets no
//! `AppState`.

use renox::state_machine::{Machine, StateMachine, Transition};

use crate::permissions;
use crate::run::{Run, RunStatus};

/// `dispatch`: the run leaves the depot.
pub const DISPATCH: &str = "dispatch";

impl StateMachine for Run {
    type State = RunStatus;
    const COLUMN: renox::db::Col<Run, RunStatus> = Run::STATUS;

    fn machine() -> Machine<Self> {
        use RunStatus::{Cancelled, Closed, Dispatched, Planned, ReturnedToDepot};
        Machine::new()
            .initial([Planned])
            .state(Planned)
            .tone("neutral")
            .state(Dispatched)
            .tone("info")
            .state(ReturnedToDepot)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(DISPATCH)
                    .label("Dispatch")
                    .tone("primary")
                    .from([Planned])
                    .to(Dispatched)
                    .permission(permissions::RUNS_DISPATCH),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_carry_their_tones_and_dispatch_leaves_planned() {
        let machine = Run::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("planned", Some("neutral"), false),
                ("dispatched", Some("info"), false),
                ("returned_to_depot", Some("warning"), false),
                ("closed", Some("success"), true),
                ("cancelled", Some("neutral"), true),
            ]
        );
        let dispatch = machine.find(DISPATCH).expect("dispatch");
        assert_eq!(dispatch.from_states(), ["planned"]);
        assert_eq!(dispatch.to_state(), Some("dispatched"));
        assert!(!dispatch.is_system());
    }
}
