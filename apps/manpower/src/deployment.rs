//! The Deployment (09 Manpower §6 *Deployment*, §7.1 *Deployment*): a worker
//! placed at a client site, with role, shift and dates.
//!
//! ```text
//! planned --activate--> active
//! active --extend--> active
//! active, suspended --end--> ended              (a user: end date, replaced)
//! active, suspended --auto_end--> ended         (the system: the end-date run)
//! planned --cancel--> cancelled                 (a user)
//! planned --auto_cancel--> cancelled            (the system: the request was cancelled)
//! active --suspend--> suspended --resume--> active   (BL-MPW-11, BL-MPW-12)
//! suspended --extend--> suspended                    (with BL-MPW-11)
//! ```
//!
//! Built here: the five states and `activate`, `extend`, `end`, `auto_end`,
//! `cancel` and `auto_cancel`. `suspended` is declared (and `end` and `auto_end`
//! leave it) so **`suspend` and `resume` come with BL-MPW-11 and BL-MPW-12**,
//! which also add `extend` of a suspended deployment.
//!
//! - `activate` (`deployments.manage` for the client) needs the employee
//!   `active` in HR (the kernel's `Workforce`), the contract `active` or
//!   `terminating`, the end date not past and, when it fills a request, that
//!   request still taking workers; the request takes one position in the same
//!   transaction (`fill_partly` or `fill`). **The document and licence check
//!   (`MPW.ACL.07`) comes with BL-MPW-08, the overlap check (`MPW.ACL.15`)
//!   with BL-MPW-10, the blocked-client check with BL-MPW-15.**
//! - `extend` moves the end date later, within the contract, while the
//!   contract takes deployments (`active` or `terminating`; the document
//!   check of the new end date comes with BL-MPW-08).
//! - `end` is the user's, with a reason: for now *end date* (the date has
//!   come) and *replaced*; **the other reasons come with their issues:
//!   *client rejected* and *client non-payment* and *entered in error* with
//!   BL-MPW-14, *contract end* with BL-MPW-06, *exit*, *absconded* and
//!   *deceased* with BL-MPW-13.** `auto_end` is the system's and takes any
//!   reason, so those issues only call it. The daily run ends the deployments
//!   whose end date has passed ([`crate::deployments::end_due`]). Closing the
//!   open timesheet of the period comes with BL-MPW-06, which owns the
//!   timesheet contract.
//! - `cancel` (a reason) is for a deployment that never started.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::db::set_deployment;
use crate::permissions;
use crate::texts::english;

/// The subject type of a deployment.
pub const SUBJECT_TYPE: &str = "manpower.deployment";

/// The `activate` transition's name.
pub const ACTIVATE: &str = "activate";
/// The `extend` transition's name.
pub const EXTEND: &str = "extend";
/// The `end` transition's name.
pub const END: &str = "end";
/// The system's `end`.
pub const AUTO_END: &str = "auto_end";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";
/// The system's `cancel`.
pub const AUTO_CANCEL: &str = "auto_cancel";

/// The reasons a deployment ends (§7.1 `end`).
pub mod reasons {
    /// The end date was reached.
    pub const END_DATE: &str = "end_date";
    /// The contract ended.
    pub const CONTRACT_END: &str = "contract_end";
    /// The client rejected the worker.
    pub const CLIENT_REJECTED: &str = "client_rejected";
    /// The MD withdrew the worker because the client did not pay.
    pub const CLIENT_NON_PAYMENT: &str = "client_non_payment";
    /// Another worker took the place.
    pub const REPLACED: &str = "replaced";
    /// The employee exited.
    pub const EXIT: &str = "exit";
    /// The employee absconded.
    pub const ABSCONDED: &str = "absconded";
    /// The employee died.
    pub const DECEASED: &str = "deceased";
    /// The deployment was a mistake.
    pub const ENTERED_IN_ERROR: &str = "entered_in_error";

    /// Every reason of the spec; the system's `auto_end` takes any.
    pub const ALL: &[&str] = &[
        END_DATE,
        CONTRACT_END,
        CLIENT_REJECTED,
        CLIENT_NON_PAYMENT,
        REPLACED,
        EXIT,
        ABSCONDED,
        DECEASED,
        ENTERED_IN_ERROR,
    ];

    /// The reasons a user may give for now; the rest come with their issues.
    pub const USER: &[&str] = &[END_DATE, REPLACED];
}

/// Where a deployment stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeploymentStatus {
    /// Agreed with the client; not started.
    #[default]
    Planned,
    /// Worker on site.
    Active,
    /// The worker may not work at the site for now (BL-MPW-11).
    Suspended,
    /// Finished (terminal).
    Ended,
    /// Never started (terminal).
    Cancelled,
}

/// What a deployment is for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeploymentKind {
    /// The worker the client asked for.
    #[default]
    Standard,
    /// A worker who stands in for another (`MPW.DEP.02`).
    Cover,
}

/// A worker placed at a client site.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manpower_deployments",
    state_machine,
    index(employee_id),
    index(client_contract_id),
    index(request_id),
    index(status)
)]
pub struct Deployment {
    /// The primary key.
    pub id: i64,
    /// The wl-hr employee.
    pub employee_id: i64,
    /// The client contract it is under.
    pub client_contract_id: i64,
    /// The client company, a wl-parties party.
    pub client_party_id: i64,
    /// The site.
    pub site_id: i64,
    /// The request it fills.
    pub request_id: Option<i64>,
    /// The role.
    pub role: String,
    /// The shift type the rate card prices.
    pub shift_type: String,
    /// The shift start as `HH:MM` (Asia/Dubai).
    pub shift_start: Option<String>,
    /// The shift end as `HH:MM`.
    pub shift_end: Option<String>,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day; none while open-ended.
    pub end_date: Option<NaiveDate>,
    /// A client's worker or a cover.
    pub kind: DeploymentKind,
    /// A cover: the deployment it stands in for.
    pub covers_deployment_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DeploymentStatus,
    /// The day it was activated.
    pub activated_on: Option<NaiveDate>,
    /// Why it ended, one of [`reasons`].
    pub end_reason: Option<String>,
    /// The day it ended.
    pub ended_on: Option<NaiveDate>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who planned it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Deployment {
    /// The deployment as a [`Subject`]: `manpower.deployment:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The employee as an [`EmployeeId`].
    pub fn employee(&self) -> EmployeeId {
        EmployeeId(self.employee_id)
    }

    /// The client as a [`PartyId`].
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The scope roles are assigned in: the client company.
    pub fn scope(&self) -> Scope {
        permissions::client_scope(self.client_party_id)
    }

    /// The scope of the site, where a site supervisor is assigned.
    pub fn site_scope(&self) -> Scope {
        permissions::site_scope(self.site_id)
    }

    /// Whether a client user sees it: the worker is or was on site.
    pub fn shown_to_client(&self) -> bool {
        matches!(
            self.status,
            DeploymentStatus::Active | DeploymentStatus::Suspended | DeploymentStatus::Ended
        )
    }

    /// Whether the worker is on site or may come back: `active` or
    /// `suspended` (the deployments of a contract "on site", §4.1).
    pub fn is_on_site(&self) -> bool {
        matches!(
            self.status,
            DeploymentStatus::Active | DeploymentStatus::Suspended
        )
    }
}

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentInput {
    /// Every check that fails, one line each, gathered by the service before
    /// the run; the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// `extend`: the new end date.
    #[serde(default)]
    pub new_end_date: Option<NaiveDate>,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> DeploymentInput {
    ctx.input_as::<DeploymentInput>().unwrap_or_default()
}

/// The guard that refuses while the service found a problem.
fn ready(transition: Transition<Deployment>) -> Transition<Deployment> {
    transition.guard("ready", |_deployment: &Deployment, ctx| {
        let problems = input_of(ctx).problems;
        Box::pin(async move {
            if problems.is_empty() {
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        })
    })
}

/// A user transition of the deployment: `deployments.manage` in the client's
/// scope.
fn user(name: &str, label: &str) -> Transition<Deployment> {
    Transition::new(name).label(label).permission_in(
        permissions::DEPLOYMENTS_MANAGE,
        |deployment: &Deployment| deployment.scope(),
    )
}

/// `end` and `auto_end`: the reason is one of `allowed`; the reason, the day
/// and the last day are kept.
fn ended(
    transition: Transition<Deployment>,
    allowed: &'static [&'static str],
) -> Transition<Deployment> {
    transition
        .reason_required()
        .from([DeploymentStatus::Active, DeploymentStatus::Suspended])
        .to(DeploymentStatus::Ended)
        .guard("known_reason", move |_deployment: &Deployment, ctx| {
            let reason = ctx.reason().map(|reason| reason.trim().to_owned());
            let known = reason
                .as_deref()
                .is_some_and(|reason| allowed.contains(&reason));
            Box::pin(async move {
                if known {
                    Ok(())
                } else {
                    Err(english("manpower.deployment.end_reason", &[]))
                }
            })
        })
        .effect(|deployment: &Deployment, ctx, tx| {
            let id = deployment.id;
            let reason = ctx.reason().map(|reason| reason.trim().to_owned());
            let on = input_of(ctx).on.unwrap_or_else(today);
            // The last day is the end date, or the day it ended when that
            // is earlier or there is none.
            let last = deployment.end_date.map_or(on, |end| end.min(on));
            Box::pin(async move {
                set_deployment(
                    &mut *tx,
                    id,
                    &[
                        ("end_reason", &reason),
                        ("ended_on", &Some(on)),
                        ("end_date", &Some(last)),
                    ],
                )
                .await
            })
        })
}

/// `cancel` and `auto_cancel`: a reason, kept.
fn cancelled(transition: Transition<Deployment>) -> Transition<Deployment> {
    transition
        .reason_required()
        .from([DeploymentStatus::Planned])
        .to(DeploymentStatus::Cancelled)
        .effect(|deployment: &Deployment, ctx, tx| {
            let id = deployment.id;
            let reason = ctx.reason().map(|reason| reason.trim().to_owned());
            Box::pin(
                async move { set_deployment(&mut *tx, id, &[("cancel_reason", &reason)]).await },
            )
        })
}

impl StateMachine for Deployment {
    type State = DeploymentStatus;
    const COLUMN: renox::db::Col<Deployment, DeploymentStatus> = Deployment::STATUS;

    fn machine() -> Machine<Self> {
        use DeploymentStatus::{Active, Cancelled, Ended, Planned, Suspended};
        Machine::new()
            .initial([Planned])
            .state(Planned)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Suspended)
            .tone("warning")
            .state(Ended)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                ready(user(ACTIVATE, "Activate").tone("primary"))
                    .from([Planned])
                    .to(Active)
                    .effect(|deployment: &Deployment, ctx, tx| {
                        let id = deployment.id;
                        let on = input_of(ctx).on.unwrap_or_else(today);
                        Box::pin(async move {
                            set_deployment(&mut *tx, id, &[("activated_on", &Some(on))]).await
                        })
                    }),
            )
            .transition(
                ready(user(EXTEND, "Extend"))
                    .from([Active])
                    .to(Active)
                    .effect(|deployment: &Deployment, ctx, tx| {
                        let id = deployment.id;
                        let end = input_of(ctx).new_end_date;
                        Box::pin(async move {
                            set_deployment(&mut *tx, id, &[("end_date", &end)]).await
                        })
                    }),
            )
            .transition(ready(ended(user(END, "End").tone("danger"), reasons::USER)))
            .transition(ended(
                Transition::new(AUTO_END).label("End").system(),
                reasons::ALL,
            ))
            .transition(ready(cancelled(user(CANCEL, "Cancel").tone("danger"))))
            .transition(cancelled(
                Transition::new(AUTO_CANCEL).label("Cancel").system(),
            ))
    }
}

/// The Deployment in the kernel's registry, with the client company as the
/// scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Deployment>(SUBJECT_TYPE)
        .scope(|deployment: &Deployment| deployment.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Deployment>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_that_is_built_is_declared() {
        let machine = Deployment::machine();
        let table: [(&str, &[&str], &str, bool); 6] = [
            (ACTIVATE, &["planned"], "active", false),
            (EXTEND, &["active"], "active", false),
            (END, &["active", "suspended"], "ended", false),
            (AUTO_END, &["active", "suspended"], "ended", true),
            (CANCEL, &["planned"], "cancelled", false),
            (AUTO_CANCEL, &["planned"], "cancelled", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert_eq!(transition.is_system(), *by_system, "{name}");
        }
        for name in [END, AUTO_END, CANCEL, AUTO_CANCEL] {
            assert!(machine.find(name).expect(name).needs_reason(), "{name}");
        }
    }

    #[test]
    fn the_states_of_the_spec_are_declared_for_the_issues_that_add_transitions() {
        let machine = Deployment::machine();
        assert_eq!(machine.initial_states(), ["planned"]);
        let names: Vec<_> = machine
            .states()
            .iter()
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["planned", "active", "suspended", "ended", "cancelled"]
        );
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|state| state.terminal)
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(terminal, ["ended", "cancelled"]);
        assert!(machine.states().iter().all(|state| state.tone.is_some()));
    }

    #[test]
    fn the_user_ends_for_two_reasons_and_the_system_for_any() {
        assert_eq!(reasons::USER, [reasons::END_DATE, reasons::REPLACED]);
        assert_eq!(reasons::ALL.len(), 9);
        assert!(reasons::USER.iter().all(|r| reasons::ALL.contains(r)));
    }

    #[test]
    fn a_worker_is_on_site_while_active_or_suspended() {
        let mut deployment = Deployment::default();
        for (status, on_site) in [
            (DeploymentStatus::Planned, false),
            (DeploymentStatus::Active, true),
            (DeploymentStatus::Suspended, true),
            (DeploymentStatus::Ended, false),
            (DeploymentStatus::Cancelled, false),
        ] {
            deployment.status = status;
            assert_eq!(deployment.is_on_site(), on_site, "{status:?}");
        }
    }

    #[test]
    fn a_deployment_is_a_subject_and_has_both_scopes() {
        let deployment = Deployment {
            id: 4,
            employee_id: 7,
            client_party_id: 9,
            site_id: 3,
            ..Deployment::default()
        };
        assert_eq!(deployment.subject().to_string(), "manpower.deployment:4");
        assert_eq!(deployment.scope(), Scope::new("manpower.client", 9));
        assert_eq!(deployment.site_scope(), Scope::new("manpower.site", 3));
        assert_eq!(deployment.employee(), EmployeeId(7));
        assert_eq!(deployment.client(), PartyId(9));
    }
}
