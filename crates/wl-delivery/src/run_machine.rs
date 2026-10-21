//! The Run state machine (wl-delivery §4.2).
//!
//! ```text
//! [*] --> planned --dispatch--> dispatched --return_to_depot--> returned_to_depot --close--> closed
//! dispatched --hand_over (the receiving driver confirms)--> dispatched
//! dispatched --abort / abort_by_driver--> returned_to_depot
//! ```
//!
//! `cancelled` is declared for the run `cancel` of a later issue.
//! `hand_over` is in [`crate::hand_over`] (BL-DLV-09), `abort` and
//! `abort_by_driver` in [`crate::abort`] (BL-DLV-08), `return_to_depot` and
//! `close` in [`crate::close`] (BL-DLV-11); the states are declared so the
//! diagram is whole. The
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
        let machine = Machine::new()
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
            );
        crate::close::transitions()
            .into_iter()
            .chain(crate::hand_over::transitions())
            .chain(crate::abort::transitions())
            .fold(machine, Machine::transition)
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

    #[test]
    fn the_run_returns_to_the_depot_by_its_driver_and_closes_by_the_system() {
        let machine = Run::machine();
        let back = machine.find(crate::close::RETURN_TO_DEPOT).expect("return");
        assert_eq!(back.from_states(), ["dispatched"]);
        assert_eq!(back.to_state(), Some("returned_to_depot"));
        assert!(!back.is_system());
        let close = machine.find(crate::close::CLOSE).expect("close");
        assert_eq!(close.from_states(), ["returned_to_depot"]);
        assert_eq!(close.to_state(), Some("closed"));
        assert!(close.is_system());
    }
}
