//! The Employee state machine (wl-hr §4, issues #216 and #194): all eight
//! states are declared. `activate` and the five non-standard exits are built
//! so far. The rest of the exit side (`give_notice`, `exit`, ...), leave and
//! the statutory items of the `activate` guard come with their own issues;
//! their events are part of the kernel contract already, because the
//! onboarding cancels on them.
//!
//! ```text
//! [*] onboarding | active
//! onboarding --activate--> active
//! onboarding --cancel_onboarding--> onboarding_cancelled
//! notice --withdraw_notice--> active
//! active | on_leave | notice --report_absconding--> absconded
//! absconded --reinstate--> active
//! active | on_leave | notice | absconded --record_death--> deceased
//! ```
//!
//! The five non-standard exits are requested by HR (`employees.manage`) and
//! run only after a second user with `employees.exit.approve` approved them
//! (`HRM.ACL.07`, [`crate::exits`]). Each is a user transition that the
//! kernel defers as an approval request, with a system twin
//! `<name>_approved` that the kernel runs at the final approval. Both carry
//! the same guard, so the documents the exit needs are checked at the
//! request and again at the approval. Their effects come with their own
//! issues (#195 to #198); this module only moves the status.
//!
//! `activate` (`employees.manage`) is for a record created in `onboarding`.
//! A minimal record has no statutory items to check (§2). A template that
//! *blocks activation* (issue #219, [`crate::activation`]) makes `activate`
//! also need every required step from that template `done` or `waived`; the
//! check runs as the first effect in the transition's transaction, because a
//! guard cannot read the database, and a refusal rolls the run back with a
//! 422 that lists the open steps and their assignees. `EmployeeActivated`
//! follows the commit.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, Transitioned};
use wl_kernel::workforce::EmployeeActivated;

use crate::activation;
use crate::employee::{Employee, EmployeeStatus};
use crate::exits::{self, ExitInput};
use crate::permissions;

/// The employee starts work.
pub const ACTIVATE: &str = "activate";

/// An onboarding that never starts ends (`HRM.ACL.07`).
pub const CANCEL_ONBOARDING: &str = "cancel_onboarding";
/// The twin of [`CANCEL_ONBOARDING`] that the kernel runs at the approval.
pub const CANCEL_ONBOARDING_APPROVED: &str = "cancel_onboarding_approved";
/// A notice is withdrawn by agreement (`HRM.ACL.07`).
pub const WITHDRAW_NOTICE: &str = "withdraw_notice";
/// The twin of [`WITHDRAW_NOTICE`] that the kernel runs at the approval.
pub const WITHDRAW_NOTICE_APPROVED: &str = "withdraw_notice_approved";
/// The employee is reported as absconded (`HRM.ACL.07`).
pub const REPORT_ABSCONDING: &str = "report_absconding";
/// The twin of [`REPORT_ABSCONDING`] that the kernel runs at the approval.
pub const REPORT_ABSCONDING_APPROVED: &str = "report_absconding_approved";
/// An absconded employee returns (`HRM.ACL.07`).
pub const REINSTATE: &str = "reinstate";
/// The twin of [`REINSTATE`] that the kernel runs at the approval.
pub const REINSTATE_APPROVED: &str = "reinstate_approved";
/// An employee's death is recorded (`HRM.ACL.07`).
pub const RECORD_DEATH: &str = "record_death";
/// The twin of [`RECORD_DEATH`] that the kernel runs at the approval.
pub const RECORD_DEATH_APPROVED: &str = "record_death_approved";

/// The user transition `name` and its system twin `approved`, from `from` to
/// `to`: both need a reason and the documents the exit names
/// ([`exits::missing`]), so the check runs at the request and again at the
/// final approval, from the payload snapshot the kernel passes as input.
fn exit_pair<const N: usize>(
    name: &'static str,
    approved: &'static str,
    label: &'static str,
    from: [EmployeeStatus; N],
    to: EmployeeStatus,
) -> [Transition<Employee>; 2] {
    let user = Transition::new(name)
        .label(label)
        .tone("danger")
        .permission(permissions::EMPLOYEES_MANAGE);
    let twin = Transition::new(approved).label(label).system();
    [user, twin].map(|transition| {
        transition
            .from(from)
            .to(to)
            .reason_required()
            .guard("exit_documents", move |_employee: &Employee, ctx| {
                let input = ctx.input_as::<ExitInput>().unwrap_or_default();
                let missing = exits::missing(name, &input);
                Box::pin(async move {
                    match missing {
                        None => Ok(()),
                        Some(message) => Err(message),
                    }
                })
            })
    })
}

impl StateMachine for Employee {
    type State = EmployeeStatus;
    const COLUMN: renox::db::Col<Employee, EmployeeStatus> = Employee::STATUS;

    fn machine() -> Machine<Self> {
        use EmployeeStatus::{
            Absconded, Active, Deceased, Exited, Notice, OnLeave, Onboarding, OnboardingCancelled,
        };
        let machine = Machine::new()
            .initial([Onboarding, Active])
            .state(Onboarding)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(OnLeave)
            .tone("info")
            .state(Notice)
            .tone("warning")
            .state(Absconded)
            .tone("error")
            .state(Deceased)
            .tone("neutral")
            .state(Exited)
            .tone("neutral")
            .terminal()
            .state(OnboardingCancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(ACTIVATE)
                    .label("Activate")
                    .tone("success")
                    .permission(permissions::EMPLOYEES_MANAGE)
                    .from([Onboarding])
                    .to(Active)
                    .effect(|employee: &Employee, _ctx, tx| {
                        let id = employee.id;
                        Box::pin(async move {
                            let blockers = activation::blockers_in(&mut *tx, id).await?;
                            if blockers.is_empty() {
                                Ok(())
                            } else {
                                Err(activation::refusal(&blockers))
                            }
                        })
                    }),
            );
        let pairs = [
            exit_pair(
                CANCEL_ONBOARDING,
                CANCEL_ONBOARDING_APPROVED,
                "Cancel onboarding",
                [Onboarding],
                OnboardingCancelled,
            ),
            exit_pair(
                WITHDRAW_NOTICE,
                WITHDRAW_NOTICE_APPROVED,
                "Withdraw notice",
                [Notice],
                Active,
            ),
            exit_pair(
                REPORT_ABSCONDING,
                REPORT_ABSCONDING_APPROVED,
                "Report absconding",
                [Active, OnLeave, Notice],
                Absconded,
            ),
            exit_pair(
                REINSTATE,
                REINSTATE_APPROVED,
                "Reinstate",
                [Absconded],
                Active,
            ),
            exit_pair(
                RECORD_DEATH,
                RECORD_DEATH_APPROVED,
                "Record death",
                [Active, OnLeave, Notice, Absconded],
                Deceased,
            ),
        ];
        pairs
            .into_iter()
            .flatten()
            .fold(machine, |machine, transition| machine.transition(transition))
    }
}

/// The machine's entry in the kernel registry: the five non-standard exits
/// are deferred as approval requests (`HRM.ACL.07`), under the action keys
/// `hr.<transition>`.
pub fn subject_machine() -> wl_kernel::SubjectMachine {
    wl_kernel::SubjectMachine::of::<Employee>(wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE).build()
}

/// Registers the machine with the kernel and the listener of `activate`.
pub(crate) fn register(app: &mut Registry) {
    subject_machine().register();
    renox::state_machine::register::<Employee>();
    app.listen(
        |event: Transitioned<Employee>, state: AppState| async move {
            if event.transition != ACTIVATE {
                return Ok(());
            }
            let Some(employee) = Employee::find(&state.db, event.key).await? else {
                return Ok(());
            };
            let scope = employee.scopes(&state.db).await?.into_iter().next();
            state
                .emit(EmployeeActivated {
                    employee: employee.employee_id(),
                    scope,
                    hire_date: employee.hire_date,
                })
                .await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_starts_in_onboarding_or_active_and_activate_is_a_user_transition() {
        let machine = Employee::machine();
        assert_eq!(machine.initial_states(), ["onboarding", "active"]);
        let activate = machine.find(ACTIVATE).unwrap();
        assert!(!activate.is_system());
        assert_eq!(activate.from_states(), ["onboarding"]);
        assert_eq!(activate.to_state(), Some("active"));
    }

    #[test]
    fn each_non_standard_exit_has_a_user_transition_and_a_system_twin() {
        let machine = Employee::machine();
        let table: [(&str, &str, &[&str], &str); 5] = [
            (
                CANCEL_ONBOARDING,
                CANCEL_ONBOARDING_APPROVED,
                &["onboarding"],
                "onboarding_cancelled",
            ),
            (WITHDRAW_NOTICE, WITHDRAW_NOTICE_APPROVED, &["notice"], "active"),
            (
                REPORT_ABSCONDING,
                REPORT_ABSCONDING_APPROVED,
                &["active", "on_leave", "notice"],
                "absconded",
            ),
            (REINSTATE, REINSTATE_APPROVED, &["absconded"], "active"),
            (
                RECORD_DEATH,
                RECORD_DEATH_APPROVED,
                &["active", "on_leave", "notice", "absconded"],
                "deceased",
            ),
        ];
        for (name, approved, from, to) in table {
            let user = machine.find(name).unwrap();
            assert!(!user.is_system(), "{name}");
            assert_eq!(user.from_states(), from, "{name}");
            assert_eq!(user.to_state(), Some(to), "{name}");
            let twin = machine.find(approved).unwrap();
            assert!(twin.is_system(), "{approved}");
            assert_eq!(twin.from_states(), from, "{approved}");
            assert_eq!(twin.to_state(), Some(to), "{approved}");
        }
        assert_eq!(machine.transitions().len(), 11);
    }
}
