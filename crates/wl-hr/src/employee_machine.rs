//! The Employee state machine (wl-hr §4, issue #216): all eight states are
//! declared, and `activate` is the one transition built so far. The exit side
//! (`give_notice`, `exit`, `cancel_onboarding`, `report_absconding`,
//! `reinstate`, `record_death`, ...), leave and the statutory items of the
//! `activate` guard come with their own issues; their events are part of the
//! kernel contract already, because the onboarding cancels on them.
//!
//! ```text
//! [*] onboarding | active
//! onboarding --activate--> active
//! ```
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
use crate::permissions;

/// The employee starts work.
pub const ACTIVATE: &str = "activate";

impl StateMachine for Employee {
    type State = EmployeeStatus;
    const COLUMN: renox::db::Col<Employee, EmployeeStatus> = Employee::STATUS;

    fn machine() -> Machine<Self> {
        use EmployeeStatus::{
            Absconded, Active, Deceased, Exited, Notice, OnLeave, Onboarding, OnboardingCancelled,
        };
        Machine::new()
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
            )
    }
}

/// Registers the listener of `activate`.
pub(crate) fn register(app: &mut Registry) {
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
    fn a_record_starts_in_onboarding_or_active_and_activate_is_the_one_transition() {
        let machine = Employee::machine();
        assert_eq!(machine.initial_states(), ["onboarding", "active"]);
        assert_eq!(machine.transitions().len(), 1);
        let activate = machine.find(ACTIVATE).unwrap();
        assert!(!activate.is_system());
        assert_eq!(activate.from_states(), ["onboarding"]);
        assert_eq!(activate.to_state(), Some("active"));
    }
}
