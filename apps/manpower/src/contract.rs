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
//! Built here: the six states and `activate` and `cancel`. `on_hold`,
//! `terminating` and `ended` are declared so the issues that add `hold`,
//! `release`, `terminate`, `renew` and `end` only add transitions: **`hold` and
//! `release` come with BL-MPW-01 and BL-MPW-03, `terminate` with BL-MPW-04,
//! `renew` with BL-MPW-05, `end` with BL-MPW-06.**
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
use wl_kernel::{PartyId, Subject, SubjectMachine};

use crate::db::set_contract;
use crate::permissions;

/// The subject type of a client contract.
pub const SUBJECT_TYPE: &str = "manpower.client_contract";

/// The `activate` transition's name.
pub const ACTIVATE: &str = "activate";
/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

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
        .scope(|contract: &ClientContract| contract.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<ClientContract>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_that_is_built_is_declared() {
        let machine = ClientContract::machine();
        let table: [(&str, &[&str], &str); 2] = [
            (ACTIVATE, &["draft"], "active"),
            (CANCEL, &["draft"], "cancelled"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in &table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), *from, "{name}");
            assert_eq!(transition.to_state(), Some(*to), "{name}");
            assert!(!transition.is_system(), "{name}");
        }
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
