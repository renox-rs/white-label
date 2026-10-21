//! The Cash handover state machine (wl-payments §4 *Cash handover*).
//!
//! ```text
//! [*] --> open --count--> counted                    (terminal)
//! open --count_short--> short
//! [*] --> short                                       (raise_shortage, a later shortage)
//! short --recount--> counted
//! short --recount_short--> short
//! short --recover--> recovered                        (terminal)
//! short --write_off--> written_off                    (terminal; `write_off_approved`
//!   when the rest is above the limit)
//! ```
//!
//! A handover is inserted `open` (the first cash collected into it,
//! [`crate::cash_custody`]) or `short` (a later shortage,
//! [`crate::cash_later`]). The transitions are those of [`crate::cash_count`]
//! and [`crate::cash_settle`]. `CashHandover` is `#[model(state_machine)]`, so
//! `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::custody::HandoverStatus;

use crate::cash_handover::CashHandover;

impl StateMachine for CashHandover {
    type State = HandoverStatus;
    const COLUMN: renox::db::Col<CashHandover, HandoverStatus> = CashHandover::STATUS;

    fn machine() -> Machine<Self> {
        use HandoverStatus::{Counted, Open, Recovered, Short, WrittenOff};
        let machine = Machine::new()
            .initial([Open, Short])
            .state(Open)
            .tone("warning")
            .state(Counted)
            .tone("success")
            .terminal()
            .state(Short)
            .tone("error")
            .state(Recovered)
            .tone("success")
            .terminal()
            .state(WrittenOff)
            .tone("neutral")
            .terminal();
        crate::cash_count::transitions()
            .into_iter()
            .chain(crate::cash_settle::transitions())
            .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_states_are_declared_and_a_handover_starts_open_or_short() {
        let machine = CashHandover::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["open", "counted", "short", "recovered", "written_off"] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["open", "short"]);
        for terminal in ["counted", "recovered", "written_off"] {
            let state = machine.states().iter().find(|s| s.name == terminal);
            assert!(state.is_some_and(|s| s.terminal), "{terminal}");
        }
    }

    #[test]
    fn the_states_have_tones() {
        let machine = CashHandover::machine();
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }

    #[test]
    fn every_transition_leads_where_the_spec_says() {
        let machine = CashHandover::machine();
        let to = |name: &str| {
            machine
                .find(name)
                .expect(name)
                .to_state()
                .map(str::to_owned)
        };
        assert_eq!(to("count").as_deref(), Some("counted"));
        assert_eq!(to("count_short").as_deref(), Some("short"));
        assert_eq!(to("recount").as_deref(), Some("counted"));
        assert_eq!(to("recount_short").as_deref(), Some("short"));
        assert_eq!(to("recover").as_deref(), Some("recovered"));
        assert_eq!(to("write_off").as_deref(), Some("written_off"));
        assert_eq!(to("write_off_approved").as_deref(), Some("written_off"));
    }
}
