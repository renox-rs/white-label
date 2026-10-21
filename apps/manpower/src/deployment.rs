//! The Deployment (09 Manpower §6 *Deployment*, §7.1 *Deployment*): a worker
//! placed at a client site, with role, shift and dates.
//!
//! ```text
//! planned --activate--> active
//! active --extend--> active
//! active --suspend--> suspended              (a user, or auto_suspend: MPW.RUL.12)
//! suspended --resume--> active               (a user, or auto_resume: MPW.RUL.12)
//! suspended --extend_suspended--> suspended  (extend of a suspended deployment)
//! active, suspended --end--> ended              (a user: end date, replaced)
//! active, suspended --auto_end--> ended         (the system: the end-date run)
//! planned --cancel--> cancelled                 (a user)
//! planned --auto_cancel--> cancelled            (the system: the request was cancelled)
//! ```
//!
//! Built here: the five states and `activate`, `extend`, `suspend`,
//! `auto_suspend`, `resume`, `auto_resume`, `end`, `auto_end`, `cancel` and
//! `auto_cancel`. A Renox transition has one `to`, so `extend` of a suspended
//! deployment is `extend_suspended`. `suspend` and `resume` are in
//! [`crate::suspension`] and [`crate::resumption`] (BL-MPW-11, BL-MPW-12): the
//! reasons are rows, the days suspended (`suspended_on`, `resumed_on`,
//! `suspended_days`) are not billed.
//!
//! - `activate` (`deployments.manage` for the client) needs the employee
//!   `active` in HR (the kernel's `Workforce`), the contract `active` or
//!   `terminating` (read in the transaction of the run; on hold it refuses with
//!   *Contract on credit hold since <date>: <reason>*, `MPW.ACL.14`), the end date not past and, when it fills a request, that
//!   request still taking workers; the request takes one position in the same
//!   transaction (`fill_partly` or `fill`). The documents and licences must be
//!   valid on the day of the action plus 30 days, or the end date if earlier
//!   (`MPW.ACL.07`, BL-MPW-08, [`crate::guard`]). A blocked client (BL-MPW-15) is
//!   refused `activate` and `extend` through [`crate::blocked`].
//!   **No overlap (`MPW.ACL.15`, BL-MPW-10, [`crate::overlap`]):** the worker
//!   has no `active` or `suspended` deployment whose shift times overlap, for
//!   `activate` and for `extend`.
//! - `extend` (`extend_suspended` for a suspended deployment, which stays
//!   suspended) moves the end date later, within the contract, while the
//!   contract takes deployments (`active` or `terminating`, not `on_hold`) and
//!   the documents and licences are valid on today plus 30 days, or the new end
//!   date if earlier (`MPW.ACL.07`).
//! - `end` is the user's, with a reason: *end date* (the date has come),
//!   *replaced*, and the three that ask for more in [`crate::ends`]: *client
//!   rejected* (the client's note), *client non-payment* (the MD's, with the
//!   notice period or a waiver) and *entered in error* (BL-MPW-14).
//!   *Contract end* is the contract's `end` (BL-MPW-06); *exit*, *absconded* and
//!   *deceased* are the system's on the exits of the worker ([`crate::exits`],
//!   BL-MPW-13). `auto_end` is the system's and takes any reason. The daily
//!   run ends the deployments whose end date has passed
//!   ([`crate::deployments::end_due`]), with the reason a scheduled end set
//!   (`planned_end_reason`) or *end date*. After every `end` the open timesheet of
//!   the worker in this deployment is closed at the last day and submitted at
//!   once, through the kernel's `TimesheetClosing` (wl-attendance,
//!   [`crate::deployments::finish_end`]).
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
/// The `suspend` transition's name.
pub const SUSPEND: &str = "suspend";
/// The system's `suspend` (`MPW.RUL.12`).
pub const AUTO_SUSPEND: &str = "auto_suspend";
/// `extend` of a suspended deployment: it stays suspended.
pub const EXTEND_SUSPENDED: &str = "extend_suspended";
/// The `resume` transition's name.
pub const RESUME: &str = "resume";
/// The system's `resume` (`MPW.RUL.12`, a renewed document accepted).
pub const AUTO_RESUME: &str = "auto_resume";
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

    /// The reasons a user may give; *contract end* is the contract's, and
    /// *exit*, *absconded* and *deceased* are the system's, on the exits of the
    /// worker. *Client non-payment* is the MD's alone.
    pub const USER: &[&str] = &[
        END_DATE,
        REPLACED,
        CLIENT_REJECTED,
        CLIENT_NON_PAYMENT,
        ENTERED_IN_ERROR,
    ];
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
    /// The weekdays of the shift, `mon,tue,wed,thu,fri,sat`; none: every day
    /// (`MPW.ACL.15`).
    pub work_days: Option<String>,
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
    /// The day the current (or the last) suspension began.
    pub suspended_on: Option<NaiveDate>,
    /// The day the last suspension ended (`resume`).
    pub resumed_on: Option<NaiveDate>,
    /// The calendar days of the suspensions that ended, not billed.
    pub suspended_days: i64,
    /// When the client, the account manager and the cover request were taken
    /// care of after the current suspension began; none while still to do.
    pub suspension_announced_at: Option<DateTime>,
    /// The last working day that notice set as the end date (`MPW.RUL.12`);
    /// none while no notice caps it.
    pub notice_last_day: Option<NaiveDate>,
    /// The end date the notice replaced; none if there was none. Meaningful
    /// while `notice_last_day` is set.
    pub end_date_before_notice: Option<NaiveDate>,
    /// When the client and the account manager were told after the system
    /// ended or cancelled it for an exit (`MPW.RUL.12`); none while to do.
    pub exit_announced_at: Option<DateTime>,
    /// Why it ended, one of [`reasons`].
    pub end_reason: Option<String>,
    /// The client's note of a rejection.
    pub end_note: Option<String>,
    /// The waiver of a short notice the MD attached (a document reference).
    pub end_waiver: Option<String>,
    /// The reason a scheduled end will be made with: the daily run ends the
    /// deployment with it instead of *end date* (client non-payment).
    pub planned_end_reason: Option<String>,
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

    /// The calendar days the worker was suspended up to `on` (the days of the
    /// suspensions that ended and, while `suspended`, the days since the
    /// current one began, both ends included). Suspended days are not billed.
    pub fn days_suspended(&self, on: NaiveDate) -> i64 {
        let current = match (self.status, self.suspended_on) {
            (DeploymentStatus::Suspended, Some(since)) if on >= since => {
                (on - since).num_days() + 1
            }
            _ => 0,
        };
        self.suspended_days + current
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
    /// `end`: the client's note, or what the user wrote.
    #[serde(default)]
    pub note: Option<String>,
    /// `end` (client non-payment): the waiver of a short notice.
    #[serde(default)]
    pub waiver: Option<String>,
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
            let input = input_of(ctx);
            let on = input.on.unwrap_or_else(today);
            let note = input
                .note
                .map(|note| note.trim().to_owned())
                .filter(|n| !n.is_empty());
            let waiver = input
                .waiver
                .map(|w| w.trim().to_owned())
                .filter(|w| !w.is_empty());
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
                        ("end_note", &note),
                        ("end_waiver", &waiver),
                    ],
                )
                .await
            })
        })
}

/// `suspend` and `auto_suspend`: a reason, from `active`; the day the
/// suspension began (the day after the expiry date for a document, else the
/// day of the action) is kept and the follow-up (the client, the account
/// manager, the cover) is marked still to do.
fn suspended(transition: Transition<Deployment>) -> Transition<Deployment> {
    transition
        .reason_required()
        .from([DeploymentStatus::Active])
        .to(DeploymentStatus::Suspended)
        .effect(|deployment: &Deployment, ctx, tx| {
            let id = deployment.id;
            let on = input_of(ctx).on.unwrap_or_else(today);
            Box::pin(async move {
                let none: Option<renox::db::DateTime> = None;
                set_deployment(
                    &mut *tx,
                    id,
                    &[
                        ("suspended_on", &Some(on)),
                        ("suspension_announced_at", &none),
                    ],
                )
                .await
            })
        })
}

/// `resume` and `auto_resume`: back to `active`; the day is kept and the
/// calendar days of the suspension are added to the days not billed.
fn resumed(transition: Transition<Deployment>) -> Transition<Deployment> {
    transition
        .from([DeploymentStatus::Suspended])
        .to(DeploymentStatus::Active)
        .effect(|deployment: &Deployment, ctx, tx| {
            let id = deployment.id;
            let on = input_of(ctx).on.unwrap_or_else(today);
            let days = deployment
                .suspended_on
                .map_or(0, |since| (on - since).num_days().max(0));
            let total = deployment.suspended_days + days;
            Box::pin(async move {
                set_deployment(
                    &mut *tx,
                    id,
                    &[("resumed_on", &Some(on)), ("suspended_days", &total)],
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
            .transition(ready(suspended(user(SUSPEND, "Suspend").tone("danger"))))
            .transition(suspended(
                Transition::new(AUTO_SUSPEND).label("Suspend").system(),
            ))
            .transition(
                ready(user(EXTEND_SUSPENDED, "Extend"))
                    .from([Suspended])
                    .to(Suspended)
                    .effect(|deployment: &Deployment, ctx, tx| {
                        let id = deployment.id;
                        let end = input_of(ctx).new_end_date;
                        Box::pin(async move {
                            set_deployment(&mut *tx, id, &[("end_date", &end)]).await
                        })
                    }),
            )
            .transition(ready(resumed(user(RESUME, "Resume").tone("primary"))))
            .transition(resumed(
                Transition::new(AUTO_RESUME).label("Resume").system(),
            ))
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
        let table: [(&str, &[&str], &str, bool); 11] = [
            (ACTIVATE, &["planned"], "active", false),
            (EXTEND, &["active"], "active", false),
            (SUSPEND, &["active"], "suspended", false),
            (AUTO_SUSPEND, &["active"], "suspended", true),
            (EXTEND_SUSPENDED, &["suspended"], "suspended", false),
            (RESUME, &["suspended"], "active", false),
            (AUTO_RESUME, &["suspended"], "active", true),
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
        for name in [SUSPEND, AUTO_SUSPEND, END, AUTO_END, CANCEL, AUTO_CANCEL] {
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
    fn the_user_ends_for_five_reasons_and_the_system_for_any() {
        assert_eq!(
            reasons::USER,
            [
                reasons::END_DATE,
                reasons::REPLACED,
                reasons::CLIENT_REJECTED,
                reasons::CLIENT_NON_PAYMENT,
                reasons::ENTERED_IN_ERROR
            ]
        );
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
