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
//! (`block` #34, `unblock` #35, `archive` / `restore` / `merge` later), all
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
        Machine::new()
            .initial([Active])
            .state(Active)
            .tone("success")
            .state(Blocked)
            .tone("error")
            .state(Archived)
            .tone("neutral")
            .state(Merged)
            .tone("neutral")
            .terminal()
    }
}
