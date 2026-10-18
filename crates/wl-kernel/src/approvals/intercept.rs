//! The kernel's transition interceptor: a user's transition that a policy
//! blocks is deferred as an approval request instead of running (wl-kernel
//! §5.2, ACCESS-CONTROL §5).
//!
//! It only looks at user runs on machines registered with
//! [`crate::machines`]. System runs always go through: approvals gate users'
//! actions, and the kernel's own `back_to_*` and `*_approved` runs must not
//! be stopped. A run into the subject's waiting state goes through too: the
//! module's effect calls [`super::submit::require_in`] itself.
//!
//! The action key of a run is the machine's (see
//! [`crate::SubjectMachine::action_key`]); the transition's input is the
//! payload the policy's condition reads.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Decision, Preview, TransitionInterceptor, TransitionRequest};

use super::policy::ApprovalPolicy;
use super::submit::{REQUEST_KEY, Require, Required, pending_for_action, require_in};
use crate::machines::{self, BoxFuture, SubjectMachine};
use crate::subject::Subject;

/// Defers users' transitions that an approval policy blocks. Installed by
/// the kernel module at boot.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApprovalInterceptor;

/// What the interceptor checks a run against: the user, the machine's
/// subject and the action key.
#[derive(Debug, PartialEq)]
struct Gated {
    user_id: i64,
    subject: Subject,
    action: String,
}

/// Which runs the interceptor checks: a user's run, on a registered
/// machine, that does not enter the machine's waiting state. `None` lets the
/// run through unchecked.
fn gate(request: &TransitionRequest) -> Option<Gated> {
    let user = request.actor.as_ref()?;
    let machine = machines::by_table(request.model)?;
    gate_on(&machine, request, user.id)
}

fn gate_on(machine: &SubjectMachine, request: &TransitionRequest, user_id: i64) -> Option<Gated> {
    if machine.waiting_state() == Some(request.to.as_str())
        || machines::is_system_name(&request.transition)
    {
        return None;
    }
    let id = request.key.parse::<i64>().ok()?;
    Some(Gated {
        user_id,
        subject: Subject::new(machine.subject_type(), id),
        action: machine.action_key(&request.transition),
    })
}

impl TransitionInterceptor for ApprovalInterceptor {
    fn intercept<'a>(
        &'a self,
        request: &'a TransitionRequest,
        tx: &'a mut Transaction,
    ) -> BoxFuture<'a, Result<Decision>> {
        Box::pin(async move {
            let Some(gated) = gate(request) else {
                return Ok(Decision::Proceed);
            };
            let require = Require::new(
                gated.action,
                gated.subject,
                request.transition.clone(),
                gated.user_id,
                request.from.clone(),
            )
            .payload(request.input.clone());
            match require_in(tx, &require).await? {
                Required::Allowed => Ok(Decision::Proceed),
                Required::Pending(created) => {
                    Ok(Decision::Defer(json!({ REQUEST_KEY: created.id })))
                }
            }
        })
    }

    fn preview<'a>(
        &'a self,
        request: &'a TransitionRequest,
        db: &'a Db,
    ) -> BoxFuture<'a, Result<Preview>> {
        Box::pin(async move {
            let Some(gated) = gate(request) else {
                return Ok(Preview::Proceed);
            };
            if pending_for_action(db, &gated.subject, &gated.action)
                .await?
                .is_some()
            {
                return Ok(Preview::Pending);
            }
            let policy = ApprovalPolicy::for_action(db, &gated.action, &request.input).await?;
            Ok(if policy.is_some() {
                Preview::RequiresApproval
            } else {
                Preview::Proceed
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use renox::state_machine::{Machine, StateMachine, Transition};
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    enum DemoStatus {
        #[default]
        Requested,
        PendingApproval,
        Paid,
    }

    #[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
    #[model(table = "demo_refunds", state_machine)]
    struct DemoRefund {
        id: i64,
        status: DemoStatus,
    }

    impl StateMachine for DemoRefund {
        type State = DemoStatus;
        const COLUMN: renox::db::Col<DemoRefund, DemoStatus> = DemoRefund::STATUS;

        fn machine() -> Machine<Self> {
            use DemoStatus::{Paid, PendingApproval, Requested};
            Machine::new()
                .state(Requested)
                .state(PendingApproval)
                .state(Paid)
                .transition(
                    Transition::new("submit")
                        .from([Requested])
                        .to(PendingApproval),
                )
                .transition(Transition::new("pay").from([Requested]).to(Paid))
                .transition(
                    Transition::new("pay_approved")
                        .from([Requested, PendingApproval])
                        .to(Paid)
                        .system(),
                )
                .transition(
                    Transition::new("back_to_requested")
                        .from([PendingApproval])
                        .to(Requested)
                        .system(),
                )
        }
    }

    fn run(transition: &str, from: &str, to: &str) -> TransitionRequest {
        TransitionRequest::new(DemoRefund::TABLE, 9, transition, from, to)
    }

    #[test]
    fn a_users_run_is_checked_under_the_machines_action_key() {
        let machine = SubjectMachine::of::<DemoRefund>("payments.refund").build();
        let gated = gate_on(&machine, &run("pay", "requested", "paid"), 2).unwrap();
        assert_eq!(
            gated,
            Gated {
                user_id: 2,
                subject: Subject::new("payments.refund", 9),
                action: "payments.pay".into(),
            }
        );
        let renamed = SubjectMachine::of::<DemoRefund>("payments.refund")
            .action("pay", "payments.refund")
            .build();
        let gated = gate_on(&renamed, &run("pay", "requested", "paid"), 2).unwrap();
        assert_eq!(gated.action, "payments.refund");
    }

    #[test]
    fn a_run_into_the_waiting_state_is_left_to_the_modules_effect() {
        let machine = SubjectMachine::of::<DemoRefund>("payments.refund")
            .waiting(DemoStatus::PendingApproval)
            .build();
        let submit = run("submit", "requested", "pending_approval");
        assert!(gate_on(&machine, &submit, 2).is_none());
        assert!(gate_on(&machine, &run("pay", "requested", "paid"), 2).is_some());
    }

    #[test]
    fn the_kernels_own_runs_are_never_checked() {
        let machine = SubjectMachine::of::<DemoRefund>("payments.refund").build();
        let back = run("back_to_requested", "pending_approval", "requested");
        assert!(gate_on(&machine, &back, 2).is_none());
        let approved = run("pay_approved", "requested", "paid");
        assert!(gate_on(&machine, &approved, 2).is_none());
        // A system run has no actor: `gate` lets it through at once.
        assert!(gate(&run("pay", "requested", "paid")).is_none());
    }

    #[test]
    #[should_panic(expected = "is not a transition of the machine")]
    fn an_action_key_for_an_unknown_transition_panics() {
        let _ = SubjectMachine::of::<DemoRefund>("payments.refund").action("ship", "payments.ship");
    }
}
