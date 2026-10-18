//! The Credit note state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --apply--> applied
//!                          applied --reopen--> issued
//! ```
//!
//! The three states are declared here with their tones, and a new credit
//! note is inserted only as `draft`. There is no terminal state: `applied`
//! is left by `reopen`. The transitions (`issue`, `apply`, `reopen`) are
//! added by the issues that build them, all in this `machine()`.
//! `CreditNote` is `#[model(state_machine)]`, so `save` refuses a changed
//! status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::invoicing::CreditNoteStatus;

use crate::credit_note::CreditNote;

impl StateMachine for CreditNote {
    type State = CreditNoteStatus;
    const COLUMN: renox::db::Col<CreditNote, CreditNoteStatus> = CreditNote::STATUS;

    fn machine() -> Machine<Self> {
        use CreditNoteStatus::{Applied, Draft, Issued};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(Applied)
            .tone("success")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credit_note_starts_as_a_draft() {
        let machine = CreditNote::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.states().len(), 3);
    }
}
