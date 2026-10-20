//! The Project (02 Construction §6, §7.1 *Project*): an awarded contract. The
//! model, its machine and the transitions the termination, the final account
//! and the release of the retention need are built here; `complete` and
//! `close` are in [`crate::completion`], as far as the retention and the
//! guarantees need them.
//!
//! ```text
//! setup --start--> in_progress
//! in_progress --hold--> on_hold --resume--> in_progress
//! in_progress --complete--> practical_completion --close--> closed
//! in_progress, on_hold --terminate--> terminated      (CON.ACL.15, BL-CON-01)
//! terminated --settle_final_account--> closed         (BL-CON-02)
//! setup --cancel--> cancelled                         (BL-CON-02)
//! ```
//!
//! - `start` (`projects.manage` in the project) needs a budget. The spec also
//!   wants a site with a geofence; sites come with the issue that builds
//!   them, and the guard with them.
//! - `hold` needs a reason, `resume` clears it. Blocking attendance while a
//!   project is on hold comes with the issue that builds the hold.
//! - `terminate` is the project manager's request (`projects.terminate`); the
//!   kernel defers it as an approval request for the MD (`projects.cancel`,
//!   `CON.ACL.15`, never the requester) and runs `terminate_approved`, which
//!   has the same guard and effect ([`crate::termination`]).
//! - `settle_final_account` and `cancel` are in [`crate::final_account`] and
//!   [`crate::cancellation`].

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::cancellation::{cancel_effects, cancel_input, cancel_problem};
use crate::completion::{
    close_effects, close_guard, complete_effects, complete_input, complete_problem,
};
use crate::final_account::{settle_effects, settle_input, settle_problem};
use crate::permissions;
use crate::termination::{terminate_effects, terminate_input, terminate_problem};
use crate::texts::english;

/// The subject type of a project.
pub const SUBJECT_TYPE: &str = "construction.project";

/// The `start` transition's name.
pub const START: &str = "start";
/// The `hold` transition's name.
pub const HOLD: &str = "hold";
/// The `resume` transition's name.
pub const RESUME: &str = "resume";
/// `terminate`: the project manager's request (`CON.ACL.15`).
pub const TERMINATE: &str = "terminate";
/// `terminate_approved`: the kernel runs it at the final approval.
pub const TERMINATE_APPROVED: &str = "terminate_approved";
/// The `settle_final_account` transition's name.
pub const SETTLE_FINAL_ACCOUNT: &str = "settle_final_account";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";
/// The `complete` transition's name.
pub const COMPLETE: &str = "complete";
/// The `close` transition's name.
pub const CLOSE: &str = "close";

/// The approval policies' action key of `terminate` (`CON.ACL.15`).
pub const TERMINATE_ACTION: &str = "construction.project_terminate";

/// Where a project stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProjectStatus {
    /// Created from an accepted quotation; budget and team being set.
    #[default]
    Setup,
    /// Work on site.
    InProgress,
    /// Suspended by the client or us.
    OnHold,
    /// Handed over; defects liability period running.
    PracticalCompletion,
    /// Stopped before completion; final account being agreed.
    Terminated,
    /// Defects liability ended and final retention invoiced, or the final
    /// account after a termination agreed and invoiced (terminal).
    Closed,
    /// Cancelled before work started (terminal).
    Cancelled,
}

/// An awarded contract. Money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "construction_projects",
    state_machine,
    unique(code),
    index(status)
)]
pub struct Project {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `TWR-01`.
    pub code: String,
    /// The name.
    pub name: String,
    /// The client or main contractor, a wl-parties party.
    pub client_party_id: i64,
    /// The contract value.
    pub contract_value: i64,
    /// The budget.
    pub budget: i64,
    /// The retention withheld on each IPA, in basis points.
    pub retention_bps: i64,
    /// The cap of the retention, if any.
    pub retention_cap: Option<i64>,
    /// The share of the first release part, in basis points.
    pub retention_first_bps: i64,
    /// Whether both retention parts become due on termination
    /// (`⚠ open: lawyer (L-37)`).
    pub retention_due_on_termination: bool,
    /// The days of the defects liability period.
    pub defects_liability_days: i64,
    /// The contract completion date; an approved extension of time moves it.
    pub completion_due_on: Option<NaiveDate>,
    /// The days after a delay event within which the contract requires
    /// notice (`CON.RUL.10`); none by default.
    pub notice_days: Option<i64>,
    /// Whether the site works on Saturdays: its calendar for `CON.RUL.08`
    /// (Sunday and the public holidays never count).
    pub works_saturdays: bool,
    /// The advance paid to the project, in fils.
    pub advance_paid: i64,
    /// What the invoiced IPAs recovered of the advance, in fils.
    pub advance_recovered: i64,
    /// The project manager, who gets the tasks of the project.
    pub manager_user_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: ProjectStatus,
    /// Why it was put on hold.
    pub hold_reason: Option<String>,
    /// Why it was terminated.
    pub termination_reason: Option<String>,
    /// The termination notice, a document reference.
    pub termination_notice: Option<String>,
    /// The last working day.
    pub termination_effective_on: Option<NaiveDate>,
    /// The user who asked for the termination.
    pub termination_requested_by: Option<i64>,
    /// When the termination took effect.
    pub terminated_at: Option<DateTime>,
    /// When the wind-down after the termination ran.
    pub wound_down_at: Option<DateTime>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The handover certificate of the practical completion, a document
    /// reference.
    pub handover_certificate: Option<String>,
    /// The day the handover certificate is dated.
    pub completed_on: Option<NaiveDate>,
    /// The last day of the defects liability period.
    pub defects_liability_ends_on: Option<NaiveDate>,
    /// When the accountant was told the retention cap was reached.
    pub retention_cap_notified_at: Option<DateTime>,
    /// When the project was closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Project {
    /// A project in `setup` for `client`, with retention due on termination.
    pub fn new(code: &str, name: &str, client: PartyId) -> Self {
        Project {
            code: code.trim().to_owned(),
            name: name.trim().to_owned(),
            client_party_id: client.0,
            retention_first_bps: 5_000,
            retention_due_on_termination: true,
            defects_liability_days: 365,
            ..Project::default()
        }
    }

    /// With a budget of `fils`.
    pub fn with_budget(mut self, fils: i64) -> Self {
        self.budget = fils;
        self
    }

    /// With a site that works on Saturdays.
    pub fn working_saturdays(mut self) -> Self {
        self.works_saturdays = true;
        self
    }

    /// With a notice period of `days` after a delay event (`CON.RUL.10`).
    pub fn with_notice_days(mut self, days: i64) -> Self {
        self.notice_days = Some(days);
        self
    }

    /// With `user` as the project manager.
    pub fn managed_by(mut self, user: i64) -> Self {
        self.manager_user_id = Some(user);
        self
    }

    /// The project as a [`Subject`]: `construction.project:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The advance paid less what the invoiced IPAs recovered of it, in fils:
    /// the advance guarantee cannot go below it.
    pub fn unrecovered_advance(&self) -> i64 {
        (self.advance_paid - self.advance_recovered).max(0)
    }

    /// The client as a [`PartyId`].
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The scope roles are assigned in: the project.
    pub fn scope(&self) -> Scope {
        Scope::new(SUBJECT_TYPE, self.id)
    }
}

/// A user transition of the project: `permission` in the project's scope.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<Project> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, |project: &Project| project.scope())
}

/// The guards and the effect `terminate` and `terminate_approved` share.
fn terminate_rules(transition: Transition<Project>) -> Transition<Project> {
    transition
        .from([ProjectStatus::InProgress, ProjectStatus::OnHold])
        .to(ProjectStatus::Terminated)
        .guard("notice_and_date", |_project: &Project, ctx| {
            let problem = terminate_problem(&terminate_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|project: &Project, ctx, tx| {
            let project = project.clone();
            let input = terminate_input(ctx);
            Box::pin(async move { terminate_effects(&mut *tx, &project, &input).await })
        })
}

impl StateMachine for Project {
    type State = ProjectStatus;
    const COLUMN: renox::db::Col<Project, ProjectStatus> = Project::STATUS;

    fn machine() -> Machine<Self> {
        use ProjectStatus::{
            Cancelled, Closed, InProgress, OnHold, PracticalCompletion, Setup, Terminated,
        };
        Machine::new()
            .initial([Setup])
            .state(Setup)
            .tone("neutral")
            .state(InProgress)
            .tone("info")
            .state(OnHold)
            .tone("warning")
            .state(PracticalCompletion)
            .tone("success")
            .state(Terminated)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                user(START, "Start", permissions::PROJECTS_MANAGE)
                    .tone("primary")
                    .from([Setup])
                    .to(InProgress)
                    .guard("budget_set", |project: &Project, _ctx| {
                        let set = project.budget > 0;
                        Box::pin(async move {
                            if set {
                                Ok(())
                            } else {
                                Err(english("construction.project.budget_needed", &[]))
                            }
                        })
                    }),
            )
            .transition(
                user(HOLD, "Hold", permissions::PROJECTS_MANAGE)
                    .reason_required()
                    .from([InProgress])
                    .to(OnHold)
                    .effect(|project: &Project, ctx, tx| {
                        let id = project.id;
                        let reason = ctx.reason().map(|reason| reason.trim().to_owned());
                        Box::pin(async move {
                            Project::query()
                                .where_(Project::ID.eq(id))
                                .update(&mut *tx, &[("hold_reason", &reason)])
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                user(RESUME, "Resume", permissions::PROJECTS_MANAGE)
                    .from([OnHold])
                    .to(InProgress)
                    .effect(|project: &Project, _ctx, tx| {
                        let id = project.id;
                        Box::pin(async move {
                            let none: Option<String> = None;
                            Project::query()
                                .where_(Project::ID.eq(id))
                                .update(&mut *tx, &[("hold_reason", &none)])
                                .await?;
                            Ok(())
                        })
                    }),
            )
            .transition(
                user(COMPLETE, "Complete", permissions::PROJECTS_MANAGE)
                    .tone("primary")
                    .from([InProgress])
                    .to(PracticalCompletion)
                    .guard("handover", |_project: &Project, ctx| {
                        let problem = complete_problem(&complete_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|project: &Project, ctx, tx| {
                        let project = project.clone();
                        let input = complete_input(ctx);
                        Box::pin(async move { complete_effects(&mut *tx, &project, &input).await })
                    }),
            )
            .transition(
                user(CLOSE, "Close", permissions::PROJECTS_MANAGE)
                    .tone("primary")
                    .from([PracticalCompletion])
                    .to(Closed)
                    .guard("defects_ended", |project: &Project, _ctx| {
                        let problem = close_guard(project);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|project: &Project, _ctx, tx| {
                        let id = project.id;
                        Box::pin(async move { close_effects(&mut *tx, id).await })
                    }),
            )
            .transition(terminate_rules(
                user(TERMINATE, "Terminate", permissions::PROJECTS_TERMINATE).tone("danger"),
            ))
            .transition(terminate_rules(
                Transition::new(TERMINATE_APPROVED)
                    .label("Terminate (approved)")
                    .system(),
            ))
            .transition(
                user(
                    SETTLE_FINAL_ACCOUNT,
                    "Settle the final account",
                    permissions::PROJECTS_MANAGE,
                )
                .tone("primary")
                .from([Terminated])
                .to(Closed)
                .guard("final_account", |_project: &Project, ctx| {
                    let problem = settle_problem(&settle_input(ctx));
                    Box::pin(async move { problem.map_or(Ok(()), Err) })
                })
                .effect(|project: &Project, _ctx, tx| {
                    let id = project.id;
                    Box::pin(async move { settle_effects(&mut *tx, id).await })
                }),
            )
            .transition(
                user(CANCEL, "Cancel", permissions::PROJECTS_CANCEL)
                    .tone("danger")
                    .reason_required()
                    .from([Setup])
                    .to(Cancelled)
                    .guard("no_work_yet", |_project: &Project, ctx| {
                        let problem = cancel_problem(&cancel_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|project: &Project, ctx, tx| {
                        let id = project.id;
                        let reason = ctx.reason().map(|reason| reason.trim().to_owned());
                        Box::pin(async move { cancel_effects(&mut *tx, id, reason).await })
                    }),
            )
    }
}

/// The default approval policy of `terminate` (`CON.ACL.15`): every request
/// needs one step by the MD (a holder of `projects.cancel`, the company admin
/// by default), never the requester.
pub fn terminate_policy() -> PolicyDefault {
    PolicyDefault::new(TERMINATE_ACTION).step_or_permission(
        [wl_kernel::COMPANY_ADMIN_ROLE],
        permissions::PROJECTS_CANCEL,
    )
}

/// The Project in the kernel's registry: its approval action for `terminate`
/// and the project as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Project>(SUBJECT_TYPE)
        .action(TERMINATE, TERMINATE_ACTION)
        .scope(|project: &Project| project.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`, and
/// the default policy of `terminate`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Project>();
    terminate_policy().register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_that_is_built_is_declared() {
        let machine = Project::machine();
        let table: [(&str, &[&str], &str, bool); 9] = [
            (START, &["setup"], "in_progress", false),
            (HOLD, &["in_progress"], "on_hold", false),
            (RESUME, &["on_hold"], "in_progress", false),
            (COMPLETE, &["in_progress"], "practical_completion", false),
            (CLOSE, &["practical_completion"], "closed", false),
            (TERMINATE, &["in_progress", "on_hold"], "terminated", false),
            (
                TERMINATE_APPROVED,
                &["in_progress", "on_hold"],
                "terminated",
                true,
            ),
            (SETTLE_FINAL_ACCOUNT, &["terminated"], "closed", false),
            (CANCEL, &["setup"], "cancelled", false),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert_eq!(transition.is_system(), *by_system, "{name}");
        }
    }

    #[test]
    fn a_project_starts_in_setup_and_two_states_are_terminal() {
        let machine = Project::machine();
        assert_eq!(machine.initial_states(), ["setup"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|state| state.terminal)
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(terminal, ["closed", "cancelled"]);
        assert!(machine.states().iter().all(|state| state.tone.is_some()));
    }

    #[test]
    fn terminated_is_not_an_end() {
        let machine = Project::machine();
        let terminated = machine
            .states()
            .iter()
            .find(|state| state.name == "terminated")
            .expect("state");
        assert!(!terminated.terminal);
        assert_eq!(terminated.tone.as_deref(), Some("warning"));
    }

    #[test]
    fn terminate_and_its_approved_twin_share_the_guard() {
        let machine = Project::machine();
        for name in [TERMINATE, TERMINATE_APPROVED] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.guard_names(), ["notice_and_date"], "{name}");
        }
        assert_eq!(wl_kernel::machines::approved(TERMINATE), TERMINATE_APPROVED);
        assert!(machine.find(CANCEL).expect("cancel").needs_reason());
    }

    #[test]
    fn the_policy_asks_the_md_for_every_termination() {
        let expected = PolicyDefault::new(TERMINATE_ACTION).step_or_permission(
            [wl_kernel::COMPANY_ADMIN_ROLE],
            permissions::PROJECTS_CANCEL,
        );
        assert_eq!(terminate_policy(), expected);
    }

    #[test]
    fn the_project_is_the_scope_roles_are_assigned_in() {
        let mut project = Project::new(" TWR-01 ", " Tower ", PartyId(9));
        project.id = 4;
        assert_eq!(project.code, "TWR-01");
        assert_eq!(project.scope(), Scope::new("construction.project", 4));
        assert_eq!(project.subject().to_string(), "construction.project:4");
        assert!(project.retention_due_on_termination);
        assert_eq!(project.client(), PartyId(9));
    }
}
