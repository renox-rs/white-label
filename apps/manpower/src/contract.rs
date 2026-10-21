//! The Client contract (09 Manpower §6 *Client contract*, §7.1 *Client
//! contract*): the agreement with a client company. Requests and deployments
//! are allowed only while it is `active` (or `terminating`, for deployments).
//!
//! ```text
//! draft --activate--> active
//! draft --cancel--> cancelled
//! active, terminating --hold--> on_hold --release--> active, terminating   (BL-MPW-01, BL-MPW-03)
//! active, on_hold --renew--> same state                                    (BL-MPW-05)
//! active, on_hold --terminate--> terminating                               (BL-MPW-04)
//! active, on_hold, terminating --end--> ended                              (BL-MPW-06)
//! ```
//!
//! Built here: the six states and `activate`, `cancel`, `hold`, `release` and `terminate`. `ended` and
//! the transitions into and out of `terminating` are declared so the issues
//! that add `renew` and `end` only add transitions: **`renew` comes with
//! BL-MPW-05, `end` with BL-MPW-06.**
//!
//! - `terminate` (`contracts.manage`, the MD approves, `MPW.ACL.17`) and its
//!   `terminate_approved` twin move an `active` or `on_hold` contract to
//!   `terminating` with a new end date ([`crate::termination`]).
//! - `release` (and `release_terminating`, back to the other state; the system's
//!   `auto_release` pair and the kernel's `*_approved` twins) ends the hold
//!   ([`crate::release`]).
//! - `hold` (`contracts.manage`, with a reason) and its twin `auto_hold` (the
//!   system's, `MPW.RUL.10`) put an `active` or `terminating` contract on
//!   credit hold; the state before, the day, who held it and the reasons are
//!   kept ([`crate::hold`]).
//!
//! - `activate` (`contracts.manage` for the client) needs the signed contract
//!   (`DOC.SIG.01`: the day and the document are recorded by
//!   [`crate::contracts::record_signed`]) and a rate card version accepted by
//!   the client and in force on the start date (the kernel's `RateCards`,
//!   wl-invoicing `FIN.RAT.01`). Every failing check is listed in one refusal.
//! - `cancel` (`contracts.manage`, with a reason) is only for a contract that
//!   was never signed into force: from `draft`.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::machines::approved;
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::db::set_contract;
use crate::hold::{hold_effects, hold_input, hold_problem};
use crate::permissions;
use crate::release::{release_effects, release_input, release_policy, release_problem};
use crate::termination::{terminate_effects, terminate_input, terminate_policy, terminate_problem};

/// The subject type of a client contract.
pub const SUBJECT_TYPE: &str = "manpower.client_contract";

/// The `activate` transition's name.
pub const ACTIVATE: &str = "activate";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";
/// The `hold` transition's name: the account manager puts the contract on
/// credit hold.
pub const HOLD: &str = "hold";
/// The system's `hold` (`MPW.RUL.10`).
pub const AUTO_HOLD: &str = "auto_hold";
/// The `terminate` transition's name: the account manager's request to end the
/// contract early, which the MD approves (`MPW.ACL.17`).
pub const TERMINATE: &str = "terminate";
/// `release` back to `active`, the account manager's, approved by the MD while
/// a hold condition remains (`MPW.ACL.14`).
pub const RELEASE: &str = "release";
/// `release` back to `terminating`: the pair of [`RELEASE`] for a contract held
/// while it was terminating.
pub const RELEASE_TERMINATING: &str = "release_terminating";
/// The system's `release` to `active`: no hold condition is left.
pub const AUTO_RELEASE: &str = "auto_release";
/// The system's `release` to `terminating`.
pub const AUTO_RELEASE_TERMINATING: &str = "auto_release_terminating";

/// Where a client contract stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContractStatus {
    /// Being negotiated; rate card proposed.
    #[default]
    Draft,
    /// Signed; requests and deployments allowed.
    Active,
    /// Credit hold: no new requests or deployments; workers on site stay.
    OnHold,
    /// Notice of early termination given; ends on the end date.
    Terminating,
    /// The end date was reached; deployments ended (terminal).
    Ended,
    /// Never signed (terminal).
    Cancelled,
}

/// How often the agency invoices the client.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BillingCycle {
    /// Every week.
    Weekly,
    /// Every two weeks.
    Fortnightly,
    /// Every month.
    #[default]
    Monthly,
}

/// An agreement with a client company.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manpower_client_contracts",
    state_machine,
    unique(reference),
    index(client_party_id),
    index(status)
)]
pub struct ClientContract {
    /// The primary key.
    pub id: i64,
    /// The reference the agency gives the contract.
    pub reference: String,
    /// The client company, a wl-parties party.
    pub client_party_id: i64,
    /// The first day of the contract.
    pub start_date: NaiveDate,
    /// The last day of the contract.
    pub end_date: NaiveDate,
    /// How often the client is invoiced.
    pub billing_cycle: BillingCycle,
    /// The days the client has to pay.
    pub payment_terms_days: i64,
    /// Whether the agency must cover leave, suspension and rejection
    /// (`MPW.DEP.02`).
    pub cover_required: bool,
    /// The whole days after which a timesheet the client does not answer is
    /// deemed approved; none by default (`HRM.RUL.15`).
    pub deemed_approval_days: Option<i64>,
    /// The client contact the timesheet escalation goes to, a party.
    pub escalation_contact_party_id: Option<i64>,
    /// The calendar days of notice an early termination or a withdrawal
    /// needs.
    pub notice_period_days: i64,
    /// The account manager of the client, who gets the notices.
    pub account_manager_user_id: Option<i64>,
    /// The day the client signed (`DOC.SIG.01`).
    pub signed_on: Option<NaiveDate>,
    /// The signed contract, a document reference.
    pub signed_document: Option<String>,
    /// The lifecycle state; changed only by a transition.
    pub status: ContractStatus,
    /// The day the contract was activated.
    pub activated_on: Option<NaiveDate>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The state before the credit hold (`active` or `terminating`), kept by
    /// `hold` so `release` goes back to it; none unless `on_hold`.
    pub status_before_hold: Option<ContractStatus>,
    /// When it was put on hold; none unless `on_hold`.
    pub held_at: Option<DateTime>,
    /// The user who held it; none when the rule did or it is not on hold.
    pub held_by: Option<i64>,
    /// The end date before an early termination moved it.
    pub original_end_date: Option<NaiveDate>,
    /// Who started the early termination: `client` or `agency`.
    pub termination_initiated_by: Option<String>,
    /// Why it was terminated.
    pub termination_reason: Option<String>,
    /// The day the notice of termination was given.
    pub termination_notice_on: Option<NaiveDate>,
    /// The waiver of a notice shorter than the notice period, a document.
    pub termination_waiver: Option<String>,
    /// The user who asked for the termination.
    pub termination_requested_by: Option<i64>,
    /// When the MD's approval made the contract `terminating`.
    pub terminated_at: Option<DateTime>,
    /// When the client was sent the confirmation of the termination.
    pub termination_announced_at: Option<DateTime>,
    /// The user who created it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClientContract {
    /// The contract as a [`Subject`]: `manpower.client_contract:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The client as a [`PartyId`].
    pub fn client(&self) -> PartyId {
        PartyId(self.client_party_id)
    }

    /// The scope roles are assigned in: the client company.
    pub fn scope(&self) -> Scope {
        permissions::client_scope(self.client_party_id)
    }

    /// Whether requests and deployments may start under it: `active` (a
    /// `terminating` contract still takes deployments, `MPW.ACL.14`).
    pub fn takes_deployments(&self) -> bool {
        matches!(
            self.status,
            ContractStatus::Active | ContractStatus::Terminating
        )
    }
}

/// What a run carries to the guards: the checks the service made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractInput {
    /// Every check that fails, one line each, gathered by the service before
    /// the run (`activate`); the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> ContractInput {
    ctx.input_as::<ContractInput>().unwrap_or_default()
}

/// A user transition of the contract: `permission` in the client's scope.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<ClientContract> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, |contract: &ClientContract| contract.scope())
}

/// `terminate` and `terminate_approved`: the guard and the effect they share
/// ([`crate::termination`]).
fn terminating(transition: Transition<ClientContract>) -> Transition<ClientContract> {
    transition
        .from([ContractStatus::Active, ContractStatus::OnHold])
        .to(ContractStatus::Terminating)
        .guard("notice_and_date", |contract: &ClientContract, ctx| {
            let problem = terminate_problem(contract, &terminate_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|contract: &ClientContract, ctx, tx| {
            let contract = contract.clone();
            let input = terminate_input(ctx);
            Box::pin(async move { terminate_effects(&mut *tx, &contract, &input).await })
        })
}

/// A release back to `target`: the state before the hold must be `target`; the
/// hold is cleared and its reasons closed.
fn releasing(
    transition: Transition<ClientContract>,
    target: ContractStatus,
) -> Transition<ClientContract> {
    transition
        .from([ContractStatus::OnHold])
        .to(target)
        .guard("state_before", move |contract: &ClientContract, _ctx| {
            let problem = release_problem(contract, target);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|contract: &ClientContract, ctx, tx| {
            let contract = contract.clone();
            let input = release_input(ctx);
            Box::pin(async move { release_effects(&mut *tx, &contract, &input).await })
        })
}

/// `hold` and `auto_hold`: the contract is `active` or `terminating`, a reason
/// is given, and the state before, the day, who held it and the reasons are
/// kept.
fn holding(transition: Transition<ClientContract>) -> Transition<ClientContract> {
    transition
        .reason_required()
        .from([ContractStatus::Active, ContractStatus::Terminating])
        .to(ContractStatus::OnHold)
        .guard("reason", |_contract: &ClientContract, ctx| {
            let problem = hold_problem(&hold_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|contract: &ClientContract, ctx, tx| {
            let contract = contract.clone();
            let input = hold_input(ctx);
            Box::pin(async move { hold_effects(&mut *tx, &contract, &input).await })
        })
}

impl StateMachine for ClientContract {
    type State = ContractStatus;
    const COLUMN: renox::db::Col<ClientContract, ContractStatus> = ClientContract::STATUS;

    fn machine() -> Machine<Self> {
        use ContractStatus::{Active, Cancelled, Draft, Ended, OnHold, Terminating};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(OnHold)
            .tone("warning")
            .state(Terminating)
            .tone("warning")
            .state(Ended)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                user(ACTIVATE, "Activate", permissions::CONTRACTS_MANAGE)
                    .tone("primary")
                    .from([Draft])
                    .to(Active)
                    .guard("ready", |_contract: &ClientContract, ctx| {
                        let problems = input_of(ctx).problems;
                        Box::pin(async move {
                            if problems.is_empty() {
                                Ok(())
                            } else {
                                Err(problems.join("\n"))
                            }
                        })
                    })
                    .effect(|contract: &ClientContract, ctx, tx| {
                        let id = contract.id;
                        let on = input_of(ctx).on.unwrap_or_else(today);
                        Box::pin(async move {
                            set_contract(&mut *tx, id, &[("activated_on", &Some(on))]).await
                        })
                    }),
            )
            .transition(holding(
                user(HOLD, "Hold", permissions::CONTRACTS_MANAGE).tone("danger"),
            ))
            .transition(holding(Transition::new(AUTO_HOLD).label("Hold").system()))
            .transition(releasing(
                user(RELEASE, "Release", permissions::CONTRACTS_MANAGE).tone("primary"),
                Active,
            ))
            .transition(releasing(
                user(
                    RELEASE_TERMINATING,
                    "Release",
                    permissions::CONTRACTS_MANAGE,
                )
                .tone("primary"),
                Terminating,
            ))
            .transition(releasing(
                Transition::new(approved(RELEASE))
                    .label("Release (approved)")
                    .system(),
                Active,
            ))
            .transition(releasing(
                Transition::new(approved(RELEASE_TERMINATING))
                    .label("Release (approved)")
                    .system(),
                Terminating,
            ))
            .transition(releasing(
                Transition::new(AUTO_RELEASE).label("Release").system(),
                Active,
            ))
            .transition(releasing(
                Transition::new(AUTO_RELEASE_TERMINATING)
                    .label("Release")
                    .system(),
                Terminating,
            ))
            .transition(terminating(
                user(TERMINATE, "Terminate", permissions::CONTRACTS_MANAGE).tone("danger"),
            ))
            .transition(terminating(
                Transition::new(approved(TERMINATE))
                    .label("Terminate (approved)")
                    .system(),
            ))
            .transition(
                user(CANCEL, "Cancel", permissions::CONTRACTS_MANAGE)
                    .tone("danger")
                    .reason_required()
                    .from([Draft])
                    .to(Cancelled)
                    .effect(|contract: &ClientContract, ctx, tx| {
                        let id = contract.id;
                        let reason = ctx.reason().map(|reason| reason.trim().to_owned());
                        Box::pin(async move {
                            set_contract(&mut *tx, id, &[("cancel_reason", &reason)]).await
                        })
                    }),
            )
    }
}

/// The Client contract in the kernel's registry, with the client company as
/// the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<ClientContract>(SUBJECT_TYPE)
        .action(RELEASE_TERMINATING, crate::release::ACTION)
        .scope(|contract: &ClientContract| contract.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<ClientContract>();
    release_policy().register();
    terminate_policy().register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_that_is_built_is_declared() {
        let machine = ClientContract::machine();
        let table: [(&str, &[&str], &str, bool); 12] = [
            (ACTIVATE, &["draft"], "active", false),
            (CANCEL, &["draft"], "cancelled", false),
            (HOLD, &["active", "terminating"], "on_hold", false),
            (AUTO_HOLD, &["active", "terminating"], "on_hold", true),
            (RELEASE, &["on_hold"], "active", false),
            (RELEASE_TERMINATING, &["on_hold"], "terminating", false),
            ("release_approved", &["on_hold"], "active", true),
            (
                "release_terminating_approved",
                &["on_hold"],
                "terminating",
                true,
            ),
            (TERMINATE, &["active", "on_hold"], "terminating", false),
            (
                "terminate_approved",
                &["active", "on_hold"],
                "terminating",
                true,
            ),
            (AUTO_RELEASE, &["on_hold"], "active", true),
            (AUTO_RELEASE_TERMINATING, &["on_hold"], "terminating", true),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert_eq!(transition.is_system(), *by_system, "{name}");
        }
        assert!(machine.find(HOLD).expect("hold").needs_reason());
        assert!(machine.find(AUTO_HOLD).expect("auto_hold").needs_reason());
    }

    #[test]
    fn the_states_of_the_spec_are_declared_for_the_issues_that_add_transitions() {
        let machine = ClientContract::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let names: Vec<_> = machine
            .states()
            .iter()
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "draft",
                "active",
                "on_hold",
                "terminating",
                "ended",
                "cancelled"
            ]
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
    fn a_cancel_needs_a_reason_and_deployments_follow_the_status() {
        assert!(
            ClientContract::machine()
                .find(CANCEL)
                .expect("cancel")
                .needs_reason()
        );
        let mut contract = ClientContract::default();
        for (status, takes) in [
            (ContractStatus::Draft, false),
            (ContractStatus::Active, true),
            (ContractStatus::OnHold, false),
            (ContractStatus::Terminating, true),
            (ContractStatus::Ended, false),
            (ContractStatus::Cancelled, false),
        ] {
            contract.status = status;
            assert_eq!(contract.takes_deployments(), takes, "{status:?}");
        }
    }

    #[test]
    fn the_client_is_the_scope_roles_are_assigned_in() {
        let contract = ClientContract {
            id: 4,
            client_party_id: 9,
            ..ClientContract::default()
        };
        assert_eq!(contract.scope(), Scope::new("manpower.client", 9));
        assert_eq!(contract.subject().to_string(), "manpower.client_contract:4");
        assert_eq!(contract.client(), PartyId(9));
    }
}
