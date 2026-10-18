//! The Invoice state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --allocate--> partially_paid | paid
//!         issued | partially_paid --mark_overdue--> overdue
//!         issued | partially_paid | overdue --write_off--> written_off
//!         … --credit--> credited (terminal)
//! ```
//!
//! Every state is declared here with its tone, and a new invoice is
//! inserted only as `draft`. The transitions are added by the issues that
//! build them (`issue`, `allocate`, `mark_overdue` and `write_off` #64,
//! the others later), all in this `machine()`. `Invoice` is
//! `#[model(state_machine)]`, so `save` refuses a changed status.

use renox::state_machine::{Machine, StateMachine};
use wl_kernel::invoicing::InvoiceStatus;

use crate::invoice::Invoice;

impl StateMachine for Invoice {
    type State = InvoiceStatus;
    const COLUMN: renox::db::Col<Invoice, InvoiceStatus> = Invoice::STATUS;

    fn machine() -> Machine<Self> {
        use InvoiceStatus::{Credited, Draft, Issued, Overdue, Paid, PartiallyPaid, WrittenOff};
        Machine::new()
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
    }
}
