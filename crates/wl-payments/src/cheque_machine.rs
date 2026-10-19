//! The Cheque state machine (wl-payments §4 *Cheque*).
//!
//! ```text
//! [*] --> received --flag_due--> due_for_deposit --deposit--> deposited
//! received --deposit--> deposited
//! deposited --clear--> cleared                       (terminal)
//! deposited --bounce--> bounced --represent--> deposited
//! bounced --settle_part--> bounced
//! bounced --refer_to_legal--> legal_case --settle_part--> legal_case
//! bounced, legal_case --replace--> replaced          (terminal)
//! bounced, legal_case --write_off--> written_off     (terminal; by a user once the
//!   invoice write-offs are made, or write_off_by_system when the last one arrives)
//! received, due_for_deposit --return--> returned     (terminal; `return` by a user,
//!   `return_by_system` when a vertical ends the agreement, [`crate::cheque_return`])
//! ```
//!
//! All nine states are declared here with their tones, and a new cheque is
//! inserted only as `received`. The transitions are added by the issues that
//! build them, all in this `machine()`: `flag_due`, `deposit` and `clear`
//! ([`crate::cheque_deposit`]), `bounce` ([`crate::cheque_bounce`]),
//! `represent` ([`crate::cheque_represent`]), `settle_part` and `replace`
//! ([`crate::cheque_settle`]), `refer_to_legal` ([`crate::cheque_legal`]);
//! `write_off` ([`crate::cheque_write_off`]) and `return` ([`crate::cheque_return`]) are built. `Cheque` is
//! `#[model(state_machine)]`, so `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::cheques::ChequeStatus;

use crate::cheque::Cheque;

impl StateMachine for Cheque {
    type State = ChequeStatus;
    const COLUMN: renox::db::Col<Cheque, ChequeStatus> = Cheque::STATUS;

    fn machine() -> Machine<Self> {
        use ChequeStatus::{
            Bounced, Cleared, Deposited, DueForDeposit, LegalCase, Received, Replaced, Returned,
            WrittenOff,
        };
        let machine = Machine::new()
            .initial([Received])
            .state(Received)
            .tone("neutral")
            .state(DueForDeposit)
            .tone("warning")
            .state(Deposited)
            .tone("info")
            .state(Cleared)
            .tone("success")
            .terminal()
            .state(Bounced)
            .tone("error")
            .state(LegalCase)
            .tone("error")
            .state(Replaced)
            .tone("neutral")
            .terminal()
            .state(WrittenOff)
            .tone("neutral")
            .terminal()
            .state(Returned)
            .tone("neutral")
            .terminal();
        crate::cheque_deposit::transitions()
            .into_iter()
            .chain(crate::cheque_bounce::transitions())
            .chain(crate::cheque_represent::transitions())
            .chain(crate::cheque_settle::transitions())
            .chain(crate::cheque_legal::transitions())
            .chain(crate::cheque_write_off::transitions())
            .chain(crate::cheque_return::transitions())
            .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_states_are_declared_and_a_cheque_starts_received() {
        let machine = Cheque::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in [
            "received",
            "due_for_deposit",
            "deposited",
            "cleared",
            "bounced",
            "legal_case",
            "replaced",
            "written_off",
            "returned",
        ] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["received"]);
        for terminal in ["cleared", "replaced", "written_off", "returned"] {
            let state = machine.states().iter().find(|s| s.name == terminal);
            assert!(state.is_some_and(|s| s.terminal), "{terminal}");
        }
    }

    #[test]
    fn the_states_have_tones() {
        let machine = Cheque::machine();
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }
}
