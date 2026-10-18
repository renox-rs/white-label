//! The Party state machine (wl-parties §4).
//!
//! ```text
//! [*] --> active --block--> blocked --unblock--> active
//!         active --archive--> archived --restore--> active | blocked
//!         active | archived --merge--> merged (terminal)
//! ```
//!
//! Every state is declared here, and a new party is inserted only as
//! `active`. The transitions are added by the issues that build them
//! (`block` and `block_by_system` #34, `unblock` #35, `archive` / `restore` / `merge` later), all
//! in this `machine()`. `Party` is `#[model(state_machine)]`, so `save`
//! refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::PartyStatus;

use crate::party::Party;

impl StateMachine for Party {
    type State = PartyStatus;
    const COLUMN: renox::db::Col<Party, PartyStatus> = Party::STATUS;

    fn machine() -> Machine<Self> {
        use PartyStatus::{Active, Archived, Blocked, Merged};
        let machine = Machine::new()
            .initial([Active])
            .state(Active)
            .tone("success")
            .state(Blocked)
            .tone("error")
            .state(Archived)
            .tone("neutral")
            .state(Merged)
            .tone("neutral")
            .terminal();
        // `block` (a user with `parties.block`) and `block_by_system`
        // (screening's `confirm_match`): active → blocked; see `crate::block`.
        let machine = crate::block::transitions()
            .into_iter()
            .fold(machine, Machine::transition);
        // `unblock` (a user with `parties.block`): blocked → active; see
        // `crate::unblock`.
        let machine = machine.transition(crate::unblock::transition());
        // `restore` and `restore_blocked` (a user with `parties.archive`):
        // archived → active | blocked; see `crate::restore`.
        crate::restore::transitions()
            .into_iter()
            .fold(machine, Machine::transition)
    }
}
