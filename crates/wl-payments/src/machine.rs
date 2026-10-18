//! The Payment state machine (wl-payments §4).
//!
//! ```text
//! [*] --> pending --record--> received
//! [*] --> received            (inserted by the cheque register and others)
//! received --reverse--> reversed (terminal)
//! ```
//!
//! Every state is declared here with its tone, and a new payment is
//! inserted only as `received` or `pending`. The transitions are added by
//! the issues that build them, all in this `machine()`: `record` and
//! `record_by_system` ([`crate::record`]); `reverse`, the gateway
//! transitions and the chargeback ones later. `Payment` is
//! `#[model(state_machine)]`, so `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::payments::PaymentStatus;

use crate::payment::Payment;

impl StateMachine for Payment {
    type State = PaymentStatus;
    const COLUMN: renox::db::Col<Payment, PaymentStatus> = Payment::STATUS;

    fn machine() -> Machine<Self> {
        use PaymentStatus::{ChargedBack, Failed, Pending, Received, Reversed};
        let machine = Machine::new()
            .initial([Received, Pending])
            .state(Pending)
            .tone("info")
            .state(Received)
            .tone("success")
            .state(Failed)
            .tone("error")
            .state(ChargedBack)
            .tone("warning")
            .state(Reversed)
            .tone("neutral")
            .terminal();
        crate::record::transitions()
            .into_iter()
            .fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_states_are_declared_and_a_payment_starts_received_or_pending() {
        let machine = Payment::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["pending", "received", "failed", "charged_back", "reversed"] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["received", "pending"]);
        let reversed = machine.states().iter().find(|s| s.name == "reversed");
        assert!(reversed.is_some_and(|s| s.terminal));
    }

    #[test]
    fn the_states_have_tones() {
        let machine = Payment::machine();
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }
}
