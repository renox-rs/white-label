//! The Supplier debit note state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --settle--> settled
//!                          issued --cancel--> cancelled
//! ```
//!
//! All four states are declared here with their tones (`settled` and
//! `cancelled` are terminal), and a new debit note is inserted only as
//! `draft`. `SupplierDebitNote` is `#[model(state_machine)]`, so `save`
//! refuses a changed status. `draft` is a service
//! ([`crate::debit_notes`]), not a transition. This part declares `issue`
//! (draft to issued, `supplier_bills.approve`) with the guard that the note
//! has its bill; the other guards and the effect come with the issue part,
//! and `settle` and `cancel` with the settle part.

use renox::state_machine::{Machine, StateMachine, Transition};
use wl_kernel::invoicing::SupplierDebitNoteStatus;
use wl_kernel::machines::SubjectMachine;

use crate::permissions;
use crate::supplier_debit_note::{SUBJECT_TYPE, SupplierDebitNote};

/// The user transition that issues a draft.
pub const ISSUE: &str = "issue";

/// The `issue` transition, for the machine.
fn issue_transition() -> Transition<SupplierDebitNote> {
    Transition::new(ISSUE)
        .label("Issue")
        .tone("primary")
        .permission(permissions::SUPPLIER_BILLS_APPROVE)
        .from([SupplierDebitNoteStatus::Draft])
        .to(SupplierDebitNoteStatus::Issued)
        .guard("linked_to_bill", |note: &SupplierDebitNote, _ctx| {
            let linked = note.bill_id.is_some();
            Box::pin(async move {
                if linked {
                    Ok(())
                } else {
                    Err("The debit note is waiting for the supplier bill.".to_owned())
                }
            })
        })
}

impl StateMachine for SupplierDebitNote {
    type State = SupplierDebitNoteStatus;
    const COLUMN: renox::db::Col<SupplierDebitNote, SupplierDebitNoteStatus> =
        SupplierDebitNote::STATUS;

    fn machine() -> Machine<Self> {
        use SupplierDebitNoteStatus::{Cancelled, Draft, Issued, Settled};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("danger")
            .terminal()
            .transition(issue_transition())
    }
}

/// Registers the machine as the subject `invoicing.supplier_debit_note`
/// and for `rnx state:diagram SupplierDebitNote`.
pub(crate) fn register() {
    SubjectMachine::of::<SupplierDebitNote>(SUBJECT_TYPE).register();
    renox::state_machine::register::<SupplierDebitNote>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::TransitionCtx;

    #[test]
    fn the_four_states_and_their_tones() {
        let machine = SupplierDebitNote::machine();
        let states: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(states, ["draft", "issued", "settled", "cancelled"]);
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["settled", "cancelled"]);
        assert_eq!(machine.find(ISSUE).unwrap().from_states().len(), 1);
    }

    #[test]
    fn the_machine_is_registered_without_a_waiting_state() {
        let machine = SubjectMachine::of::<SupplierDebitNote>(SUBJECT_TYPE).build();
        assert_eq!(machine.subject_type(), "invoicing.supplier_debit_note");
        assert_eq!(machine.waiting_state(), None);
        assert!(machine.transition(ISSUE).is_some());
    }

    #[renox::test]
    async fn a_note_waiting_for_its_bill_cannot_be_issued() {
        let issue = SupplierDebitNote::machine();
        let issue = issue.find(ISSUE).unwrap();
        let ctx = TransitionCtx::new(None);
        let waiting = SupplierDebitNote::default();
        let refused = issue.check_guards(&waiting, &ctx).await.unwrap_err();
        assert!(refused.to_string().contains("waiting"));
        let linked = SupplierDebitNote {
            bill_id: Some(3),
            ..Default::default()
        };
        assert!(issue.check_guards(&linked, &ctx).await.is_ok());
    }
}
