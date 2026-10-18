//! The Invoice state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --allocate--> partially_paid | paid
//!         issued | partially_paid --mark_overdue--> overdue
//!         issued | partially_paid | overdue --write_off--> written_off
//!         written_off --recover--> written_off
//!         written_off --recover_in_full--> paid
//!         … --credit--> credited (terminal)
//! ```
//!
//! Every state is declared here with its tone, and a new invoice is
//! inserted only as `draft`. The transitions are added by the issues that
//! build them, all in this `machine()`: `issue` ([`crate::issue`]),
//! `allocate` / `allocate_in_full` ([`crate::allocate`]) and `mark_overdue`
//! ([`crate::overdue`]), `write_off` / `write_off_approved`
//! ([`crate::write_off`]), `recover` / `recover_in_full`
//! ([`crate::recover`]), `credit` ([`crate::credit`]); the others later.
//! `Invoice` is
//! `#[model(state_machine)]`, so `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::invoicing::InvoiceStatus;

use crate::invoice::Invoice;

impl StateMachine for Invoice {
    type State = InvoiceStatus;
    const COLUMN: renox::db::Col<Invoice, InvoiceStatus> = Invoice::STATUS;

    fn machine() -> Machine<Self> {
        use InvoiceStatus::{Credited, Draft, Issued, Overdue, Paid, PartiallyPaid, WrittenOff};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(PartiallyPaid)
            .tone("info")
            .state(Overdue)
            .tone("error")
            .state(Paid)
            .tone("success")
            .state(WrittenOff)
            .tone("neutral")
            .state(Credited)
            .tone("neutral")
            .terminal()
            .transition(crate::issue::transition())
            .transition(crate::overdue::transition());
        crate::allocate::transitions()
            .into_iter()
            .chain(crate::write_off::transitions())
            .chain(crate::recover::transitions())
            .chain(crate::credit::transitions())
            .fold(machine, Machine::transition)
    }
}
