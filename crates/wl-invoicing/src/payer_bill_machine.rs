//! The Payer bill state machine (wl-invoicing §4).
//!
//! ```text
//! [*] --> draft --issue--> issued --submission_rejected--> draft
//!                          issued --record_remittance--> paid | partially_paid | denied
//!         paid | partially_paid | closed --record_recoupment--> partially_paid | denied
//!         partially_paid | denied --resubmit--> issued
//!         draft | partially_paid | denied --transfer_to_customer | write_off--> closed
//!         draft --void--> voided
//! ```
//!
//! All seven states are declared here with their tones (`voided` is
//! terminal). `PayerBill` is `#[model(state_machine)]`, so `save` refuses a
//! changed status. The transitions come from one module each, which also
//! holds the service that gathers what its guards need (the machine has no
//! app state): [`crate::payer_issue`] (`issue`, `resubmit`),
//! [`crate::payer_rejection`] (`submission_rejected`),
//! [`crate::payer_remittance`] (`record_remittance`),
//! [`crate::payer_recoupment`] (`record_recoupment`) and
//! [`crate::payer_closing`] (`transfer_to_customer`, `write_off`, `void`).
//! What other modules must hear after a commit is done by listeners on
//! `Transitioned<PayerBill>` ([`register_events`]), never inside an effect.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, Transitioned};
use wl_kernel::machines::SubjectMachine;
use wl_kernel::payer_bills::PayerBillStatus;

use crate::payer_bill::{PayerBill, SUBJECT_TYPE};
use crate::payer_issue;
use crate::payer_policy;
use crate::payer_recoupment;
use crate::payer_rejection;
use crate::payer_remittance;

/// Every transition of the machine, in a stable order.
fn all_transitions() -> Vec<Transition<PayerBill>> {
    let mut all = Vec::new();
    all.extend(payer_issue::transitions());
    all.extend(payer_rejection::transitions());
    all.extend(payer_remittance::transitions());
    all.extend(payer_recoupment::transitions());
    all
}

impl StateMachine for PayerBill {
    type State = PayerBillStatus;
    const COLUMN: renox::db::Col<PayerBill, PayerBillStatus> = PayerBill::STATUS;

    fn machine() -> Machine<Self> {
        use PayerBillStatus::{Closed, Denied, Draft, Issued, PartiallyPaid, Paid, Voided};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Issued)
            .tone("info")
            .state(PartiallyPaid)
            .tone("warning")
            .state(Denied)
            .tone("danger")
            .state(Paid)
            .tone("success")
            .state(Closed)
            .tone("neutral")
            .state(Voided)
            .tone("neutral")
            .terminal();
        all_transitions()
            .into_iter()
            .fold(machine, Machine::transition)
    }
}

/// Registers the machine as the subject `invoicing.payer_bill` and for
/// `rnx state:diagram PayerBill`.
pub(crate) fn register() {
    // The overpayment of a remittance and a recoupment share one approval
    // action (`FIN.ACL.16`); a recoupment from a reversed payment is the
    // system's and is not gated.
    let mut machine = SubjectMachine::of::<PayerBill>(SUBJECT_TYPE);
    for outcome in payer_remittance::OUTCOMES {
        machine = machine.action(
            &payer_remittance::transition_name(outcome, false),
            payer_remittance::ACTION,
        );
    }
    for outcome in payer_recoupment::OUTCOMES {
        machine = machine.action(
            &payer_recoupment::transition_name(outcome, false),
            payer_remittance::ACTION,
        );
    }
    machine.register();
    renox::state_machine::register::<PayerBill>();
    payer_policy::money_policy().register();
    wl_kernel::approvals::register_subject_link(SUBJECT_TYPE, |id| format!("/payer-bills/{id}"));
}

/// Registers the listener that follows every committed transition of a
/// payer bill: the tax documents, the tasks, the notices and the events.
pub(crate) fn register_events(app: &mut Registry) {
    // A reversed payment takes back what it was applied to on a payer bill.
    app.listen(
        |event: wl_kernel::payments::PaymentReversed, state: AppState| async move {
            payer_recoupment::on_payment_reversed(&state, &event).await
        },
    );
    app.listen(|event: Transitioned<PayerBill>, state: AppState| async move {
        payer_issue::on_transition(&state, &event).await?;
        payer_rejection::on_transition(&state, &event).await?;
        payer_remittance::on_transition(&state, &event).await?;
        payer_recoupment::on_transition(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seven_states_and_their_tones() {
        let machine = PayerBill::machine();
        let states: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            states,
            [
                "draft",
                "issued",
                "partially_paid",
                "denied",
                "paid",
                "closed",
                "voided"
            ]
        );
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["voided"]);
    }

    #[test]
    fn the_machine_is_registered_as_the_payer_bill_subject() {
        let machine = SubjectMachine::of::<PayerBill>(SUBJECT_TYPE).build();
        assert_eq!(machine.subject_type(), "invoicing.payer_bill");
        assert!(machine.transition(payer_issue::ISSUE).is_some());
    }
}
