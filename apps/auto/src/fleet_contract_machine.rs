//! The fleet contract machine (08 §7.1 *Fleet contract*; `AUT.FLT.01`,
//! `AUT.RUL.04`, `AUT.RUL.05`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`ACTIVATE`] | draft | active | `fleet_contracts.manage` | `vehicles_set`, `rates_set`, `schedule_set`, `signed_contract` |
//! | [`SUSPEND`] | active, renewal_due | suspended | `fleet_contracts.manage` | a reason |
//! | [`RESUME`] | suspended | active | `fleet_contracts.manage` | — |
//! | [`FLAG_RENEWAL`] | active | renewal_due | system | `end_within_window` |
//! | [`RENEW`] | renewal_due | active | `fleet_contracts.manage` | `new_end_date`, `rates_set` |
//! | [`END`] | active, suspended, renewal_due | ended | system (daily job) | `end_date_passed` |
//! | [`TERMINATE`] / [`TERMINATE_APPROVED`] | active, suspended, renewal_due | ended | `fleet_contracts.manage`, approved by the owner (`PLT.APR.01`) / the kernel after the approval | a reason; `termination_dates`, `not_scheduled`, `owner_approval` |
//! | [`SCHEDULE_TERMINATION`] and its two twins by state, each with an `_approved` run | active / suspended / renewal_due | the same state | as above | as above |
//! | [`TERMINATE_SYSTEM`] | active, suspended, renewal_due | ended | system (daily job, after the effective date) | a reason; `termination_dates`, `owner_approval` (the termination is scheduled) |
//!
//! Decisions:
//!
//! - As for the job card, the guards see the contract and the run's input;
//!   [`crate::fleet_contracts`] works out what a guard needs from the
//!   database (the vehicles and schedules set, the days to the end) and passes
//!   it in [`FleetInput`].
//! - `activate` guards what 08 asks: vehicles, rates and the schedule set, the
//!   signed contract attached. Its effect (the service schedules created,
//!   `WRK.PPM.01`) is the service's, after the commit.
//! - `end` and `terminate` are declared with the effects of the state change
//!   only (when it ended, the termination recorded); what follows an end -
//!   future bookings cancelled, the last invoice drafted, the fleet contact
//!   told, open job cards finished - is [`crate::fleet_end`]'s, after the
//!   commit (BL-AUT-13 for `end`, BL-AUT-12 for a termination).
//! - **Termination is the owner's approval** (`PLT.APR.01`, the action
//!   [`TERMINATE_ACTION`], BL-AUT-12): the manager's request is deferred by the
//!   kernel as a request that a holder of `fleet_contracts.approve` other than
//!   the requester decides, and the contract keeps its state until then. At
//!   the approval the kernel runs `<request>_approved` with the input the
//!   request kept. A transition has one end state, and 08 says the contract
//!   *keeps its state* when the effective date is not before today (the daily
//!   job ends it on the first run after that day) and *ends at the approval*
//!   when it is: so the service picks the request by the effective date at the
//!   time of the request - [`TERMINATE`] (ends at the approval) for a date
//!   before today, [`SCHEDULE_TERMINATION`] (records the termination and marks
//!   it *scheduled*, per state because a transition keeps one state) otherwise,
//!   and, when a scheduled termination is approved after its effective date
//!   passed, the approval's listener runs [`TERMINATE_SYSTEM`] at once.
//!   `terminate_system` is the daily job's run on the first day after the
//!   effective date; its guard `owner_approval` asks that the termination was
//!   approved (it is scheduled).
//! - The guard `owner_approval` of the user's run asks that the service found
//!   the approval policy in force, so a policy taken away cannot let a
//!   termination through unapproved; the approved run's asks that the request
//!   names who asked.
//! - A suspended contract keeps its vehicles and its job cards; new bookings
//!   are not requested while it is suspended ([`crate::fleet_services`]) and a
//!   new booking for its vehicles is refused ([`crate::fleet_gate`]).

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use wl_kernel::SubjectMachine;
use wl_kernel::approvals::{self, PolicyDefault};

use crate::db::set_fleet_contract;
use crate::fleet_contract::{FLEET_CONTRACT_SUBJECT_TYPE, FleetContract, FleetStatus};
use crate::permissions;
use crate::texts::english;

/// `draft → active`: the contract is in force.
pub const ACTIVATE: &str = "activate";
/// `active, renewal_due → suspended`: new bookings are blocked.
pub const SUSPEND: &str = "suspend";
/// `suspended → active`.
pub const RESUME: &str = "resume";
/// `active → renewal_due`: the end date is within the renewal window.
pub const FLAG_RENEWAL: &str = "flag_renewal";
/// `renewal_due → active`: a new end date and rates.
pub const RENEW: &str = "renew";
/// `active, suspended, renewal_due → ended`: the end date has passed.
pub const END: &str = "end";
/// `active, suspended, renewal_due → ended`: terminated early, by a user.
pub const TERMINATE: &str = "terminate";
/// `active, suspended, renewal_due → ended`: the kernel runs it after the owner
/// approved a termination whose effective date was before the day of the
/// request.
pub const TERMINATE_APPROVED: &str = "terminate_approved";
/// `active → active`: a termination is requested; at the owner's approval it
/// is recorded and the contract is marked *termination scheduled*.
pub const SCHEDULE_TERMINATION: &str = "schedule_termination";
/// `suspended → suspended`: as [`SCHEDULE_TERMINATION`].
pub const SCHEDULE_TERMINATION_SUSPENDED: &str = "schedule_termination_suspended";
/// `renewal_due → renewal_due`: as [`SCHEDULE_TERMINATION`].
pub const SCHEDULE_TERMINATION_RENEWAL: &str = "schedule_termination_renewal_due";
/// The kernel's run after the owner approved [`SCHEDULE_TERMINATION`].
pub const SCHEDULE_TERMINATION_APPROVED: &str = "schedule_termination_approved";
/// The kernel's run after the owner approved [`SCHEDULE_TERMINATION_SUSPENDED`].
pub const SCHEDULE_TERMINATION_SUSPENDED_APPROVED: &str = "schedule_termination_suspended_approved";
/// The kernel's run after the owner approved [`SCHEDULE_TERMINATION_RENEWAL`].
pub const SCHEDULE_TERMINATION_RENEWAL_APPROVED: &str = "schedule_termination_renewal_due_approved";
/// `active, suspended, renewal_due → ended`: terminated early, by the daily
/// job on the first day after the effective date.
pub const TERMINATE_SYSTEM: &str = "terminate_system";
/// `active, renewal_due → suspended`: the owner's decision in the review of a
/// blocked fleet (BL-AUT-17), run by the system for the decider.
pub const SUSPEND_BY_DECISION: &str = "suspend_by_decision";
/// `active, suspended, renewal_due → ended`: the owner's decision in the review
/// of a blocked fleet (BL-AUT-17); the decision is the owner's approval.
pub const TERMINATE_BY_DECISION: &str = "terminate_by_decision";
/// The approval action of a termination (`PLT.APR.01`, BL-AUT-12): the owner
/// decides, whichever of the requests was made.
pub const TERMINATE_ACTION: &str = "auto.fleet_terminate";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetInput {
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// How many vehicles are under the contract ([`ACTIVATE`]).
    #[serde(default)]
    pub vehicles: Option<i64>,
    /// How many service plans the contract has ([`ACTIVATE`]).
    #[serde(default)]
    pub schedules: Option<i64>,
    /// Days from the day of the run to the end date ([`FLAG_RENEWAL`]).
    #[serde(default)]
    pub days_to_end: Option<i64>,
    /// The new last day of the term ([`RENEW`]).
    #[serde(default)]
    pub new_end_date: Option<NaiveDate>,
    /// The new hourly labour rate in fils ([`RENEW`]); the old one when none.
    #[serde(default)]
    pub labour_rate: Option<i64>,
    /// When notice of the termination was given ([`TERMINATE`]).
    #[serde(default)]
    pub notice_date: Option<NaiveDate>,
    /// The last day of service ([`TERMINATE`]).
    #[serde(default)]
    pub effective_date: Option<NaiveDate>,
    /// Why the contract is terminated: the kernel hands it to the approved run.
    #[serde(default)]
    pub reason: Option<String>,
    /// The fleet asked for the early termination fee to be waived; the owner's
    /// approval waives it ([`TERMINATE`]).
    #[serde(default)]
    pub waive_fee: bool,
    /// The service found the approval policy of the termination in force.
    #[serde(default)]
    pub approval_ready: bool,
}

fn input_of(ctx: &TransitionCtx) -> FleetInput {
    ctx.input_as::<FleetInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// The labour rate is agreed (`AUT.FLT.01`).
fn with_rates(transition: Transition<FleetContract>) -> Transition<FleetContract> {
    transition.guard("rates_set", |c: &FleetContract, ctx| {
        let ok = input_of(ctx).labour_rate.unwrap_or(c.labour_rate) > 0;
        Box::pin(async move { check(ok, "auto.fleet.problem.rates") })
    })
}

/// The notice and effective dates of a termination: both given, the effective
/// date not before the notice date nor inside an invoiced period.
fn with_termination_dates(transition: Transition<FleetContract>) -> Transition<FleetContract> {
    transition.guard("termination_dates", |c: &FleetContract, ctx| {
        let input = input_of(ctx);
        let result = match (input.notice_date, input.effective_date) {
            (Some(notice), Some(effective)) if effective < notice => {
                Err(english("auto.fleet.problem.effective_before_notice", &[]))
            }
            (Some(_), Some(effective))
                if c.invoiced_through_date
                    .is_some_and(|through| effective <= through) =>
            {
                Err(english("auto.fleet.problem.effective_invoiced", &[]))
            }
            (Some(_), Some(_)) => Ok(()),
            _ => Err(english("auto.fleet.problem.termination_dates", &[])),
        };
        Box::pin(async move { result })
    })
}

impl StateMachine for FleetContract {
    type State = FleetStatus;
    const COLUMN: renox::db::Col<FleetContract, FleetStatus> = FleetContract::STATUS;

    fn machine() -> Machine<Self> {
        use FleetStatus::{Active, Draft, Ended, RenewalDue, Suspended};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Suspended)
            .tone("warning")
            .state(RenewalDue)
            .tone("warning")
            .state(Ended)
            .tone("neutral")
            .terminal()
            .transition(activate())
            .transition(suspension(SUSPEND, false))
            .transition(suspension(SUSPEND_BY_DECISION, true))
            .transition(resume())
            .transition(flag_renewal())
            .transition(renew())
            .transition(end())
            .transition(termination(
                TERMINATE,
                &IN_FORCE,
                FleetStatus::Ended,
                Run::User,
            ))
            .transition(termination(
                TERMINATE_APPROVED,
                &IN_FORCE,
                FleetStatus::Ended,
                Run::Approved,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION,
                &[Active],
                Active,
                Run::User,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION_APPROVED,
                &[Active],
                Active,
                Run::Approved,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION_SUSPENDED,
                &[Suspended],
                Suspended,
                Run::User,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION_SUSPENDED_APPROVED,
                &[Suspended],
                Suspended,
                Run::Approved,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION_RENEWAL,
                &[RenewalDue],
                RenewalDue,
                Run::User,
            ))
            .transition(termination(
                SCHEDULE_TERMINATION_RENEWAL_APPROVED,
                &[RenewalDue],
                RenewalDue,
                Run::Approved,
            ))
            .transition(termination(
                TERMINATE_SYSTEM,
                &IN_FORCE,
                FleetStatus::Ended,
                Run::System,
            ))
            .transition(termination(
                TERMINATE_BY_DECISION,
                &IN_FORCE,
                FleetStatus::Ended,
                Run::Decision,
            ))
    }
}

/// The states in which a contract is in force.
const IN_FORCE: [FleetStatus; 3] = [
    FleetStatus::Active,
    FleetStatus::Suspended,
    FleetStatus::RenewalDue,
];

/// Who runs a termination transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    /// The fleet account manager; the kernel defers it for the owner.
    User,
    /// The kernel, after the owner approved.
    Approved,
    /// The daily job, after the effective date.
    System,
    /// The owner's decision in the review of a blocked fleet: the decision is
    /// the approval, and the termination is recorded and ends at once.
    Decision,
}

/// `activate`: vehicles, rates and the schedule are set and the signed
/// contract is attached; the service schedules are created after the commit
/// (`WRK.PPM.01`).
fn activate() -> Transition<FleetContract> {
    let transition = Transition::new(ACTIVATE)
        .label("Activate")
        .tone("primary")
        .permission(permissions::FLEET_CONTRACTS_MANAGE)
        .from([FleetStatus::Draft])
        .to(FleetStatus::Active)
        .guard("vehicles_set", |_c: &FleetContract, ctx| {
            let ok = input_of(ctx).vehicles.unwrap_or(0) > 0;
            Box::pin(async move { check(ok, "auto.fleet.problem.vehicles") })
        });
    with_rates(transition)
        .guard("schedule_set", |_c: &FleetContract, ctx| {
            let ok = input_of(ctx).schedules.unwrap_or(0) > 0;
            Box::pin(async move { check(ok, "auto.fleet.problem.schedule") })
        })
        .guard("signed_contract", |c: &FleetContract, _ctx| {
            let ok = c
                .signed_document
                .as_deref()
                .is_some_and(|d| !d.trim().is_empty());
            Box::pin(async move { check(ok, "auto.fleet.problem.signed") })
        })
        .effect(|c: &FleetContract, _ctx, tx| {
            let id = c.id;
            Box::pin(
                async move { set_fleet_contract(tx, id, &[("activated_at", &Some(now()))]).await },
            )
        })
}

/// `suspend`: new bookings are blocked; the bookings already made are kept and
/// open job cards finish. The system's twin is the owner's decision in the
/// review of a blocked fleet (BL-AUT-17).
fn suspension(name: &'static str, system: bool) -> Transition<FleetContract> {
    let transition = Transition::new(name)
        .label(if system {
            "Suspend (decision)"
        } else {
            "Suspend"
        })
        .from([FleetStatus::Active, FleetStatus::RenewalDue])
        .to(FleetStatus::Suspended)
        .reason_required();
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::FLEET_CONTRACTS_MANAGE)
    };
    transition.effect(|c: &FleetContract, ctx, tx| {
        let id = c.id;
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            set_fleet_contract(
                tx,
                id,
                &[
                    ("suspended_reason", &reason),
                    ("suspended_at", &Some(now())),
                ],
            )
            .await
        })
    })
}

/// `resume`: the contract is in force again.
fn resume() -> Transition<FleetContract> {
    Transition::new(RESUME)
        .label("Resume")
        .tone("primary")
        .permission(permissions::FLEET_CONTRACTS_MANAGE)
        .from([FleetStatus::Suspended])
        .to(FleetStatus::Active)
        .effect(|c: &FleetContract, _ctx, tx| {
            let id = c.id;
            Box::pin(async move {
                let none_text: Option<String> = None;
                let none_time: Option<renox::db::DateTime> = None;
                set_fleet_contract(
                    tx,
                    id,
                    &[
                        ("suspended_reason", &none_text),
                        ("suspended_at", &none_time),
                    ],
                )
                .await
            })
        })
}

/// `flag_renewal`: the end date is within the renewal window.
fn flag_renewal() -> Transition<FleetContract> {
    Transition::new(FLAG_RENEWAL)
        .label("Renewal due")
        .system()
        .from([FleetStatus::Active])
        .to(FleetStatus::RenewalDue)
        .guard("end_within_window", |c: &FleetContract, ctx| {
            let days = input_of(ctx).days_to_end;
            let ok = days.is_some_and(|days| days <= c.renewal_window_days);
            Box::pin(async move { check(ok, "auto.fleet.problem.not_in_window") })
        })
}

/// `renew`: a new end date, and the rates when they change.
fn renew() -> Transition<FleetContract> {
    let transition = Transition::new(RENEW)
        .label("Renew")
        .tone("primary")
        .permission(permissions::FLEET_CONTRACTS_MANAGE)
        .from([FleetStatus::RenewalDue])
        .to(FleetStatus::Active)
        .guard("new_end_date", |c: &FleetContract, ctx| {
            let ok = input_of(ctx)
                .new_end_date
                .is_some_and(|end| end > c.end_date);
            Box::pin(async move { check(ok, "auto.fleet.problem.new_end") })
        });
    with_rates(transition).effect(|c: &FleetContract, ctx, tx| {
        let contract = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            let end = input.new_end_date.unwrap_or(contract.end_date);
            let rate = input.labour_rate.unwrap_or(contract.labour_rate);
            set_fleet_contract(
                tx,
                contract.id,
                &[("end_date", &end), ("labour_rate", &rate)],
            )
            .await
        })
    })
}

/// `end`: the end date has passed; the daily job ends the contract on the
/// first run after it (the contract is valid through its end date).
fn end() -> Transition<FleetContract> {
    Transition::new(END)
        .label("End")
        .system()
        .from([
            FleetStatus::Active,
            FleetStatus::Suspended,
            FleetStatus::RenewalDue,
        ])
        .to(FleetStatus::Ended)
        .guard("end_date_passed", |c: &FleetContract, ctx| {
            let ok = input_of(ctx).on.is_some_and(|day| day > c.end_date);
            Box::pin(async move { check(ok, "auto.fleet.problem.not_over") })
        })
        .effect(|c: &FleetContract, _ctx, tx| {
            let id = c.id;
            Box::pin(async move { set_fleet_contract(tx, id, &[("ended_at", &Some(now()))]).await })
        })
}

/// A termination: the contract ends early with a reason, a notice date and an
/// effective date (the last day of service), or, for the requests that keep
/// the state, is recorded and marked *termination scheduled*.
fn termination(
    name: &'static str,
    from: &[FleetStatus],
    to: FleetStatus,
    who: Run,
) -> Transition<FleetContract> {
    let ends = to == FleetStatus::Ended;
    let label = match who {
        Run::User if ends => "Terminate",
        Run::User => "Request termination",
        Run::Approved => "Terminate (approved)",
        Run::System => "Terminate (effective date passed)",
        Run::Decision => "Terminate (decision)",
    };
    let transition = Transition::new(name)
        .label(label)
        .from(from.iter().copied())
        .to(to)
        .reason_required();
    let transition = if who == Run::User {
        transition.permission(permissions::FLEET_CONTRACTS_MANAGE)
    } else {
        transition.system()
    };
    let transition = with_termination_dates(transition);
    let transition = if matches!(who, Run::System | Run::Decision) {
        transition
    } else {
        transition.guard("not_scheduled", |c: &FleetContract, _ctx| {
            let ok = !c.termination_scheduled;
            Box::pin(async move { check(ok, "auto.fleet.problem.already_scheduled") })
        })
    };
    transition
        .guard("owner_approval", move |c: &FleetContract, ctx| {
            let input = input_of(ctx);
            let ok = match who {
                Run::User => input.approval_ready,
                Run::Approved | Run::Decision => input.by.is_some(),
                Run::System => c.termination_scheduled,
            };
            Box::pin(async move { check(ok, "auto.fleet.problem.approval") })
        })
        .effect(move |c: &FleetContract, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                if who != Run::System {
                    let approved_at = matches!(who, Run::Approved | Run::Decision).then(now);
                    let approved_by = if who == Run::Decision { input.by } else { None };
                    set_fleet_contract(
                        tx,
                        id,
                        &[
                            ("termination_reason", &reason),
                            ("notice_date", &input.notice_date),
                            ("effective_date", &input.effective_date),
                            ("termination_scheduled", &!ends),
                            ("termination_requested_by", &input.by),
                            ("termination_fee_waived", &input.waive_fee),
                            ("termination_approved_at", &approved_at),
                            ("termination_approved_by", &approved_by),
                        ],
                    )
                    .await?;
                }
                if ends {
                    set_fleet_contract(
                        tx,
                        id,
                        &[
                            ("termination_scheduled", &false),
                            ("ended_at", &Some(now())),
                        ],
                    )
                    .await?;
                }
                Ok(())
            })
        })
}

/// The default approval policy of a termination (`PLT.APR.01`, BL-AUT-12): one
/// step by a holder of `fleet_contracts.approve` (the owner), never the fleet
/// account manager who asked. The decision also waives the early termination
/// fee when the request asked for it.
pub fn terminate_policy() -> PolicyDefault {
    PolicyDefault::new(TERMINATE_ACTION).step_permission(permissions::FLEET_CONTRACTS_APPROVE)
}

/// The names of the requests that keep the state, by the state they are made
/// in, with the run the kernel makes after the approval.
pub fn schedule_request(status: FleetStatus) -> Option<&'static str> {
    match status {
        FleetStatus::Active => Some(SCHEDULE_TERMINATION),
        FleetStatus::Suspended => Some(SCHEDULE_TERMINATION_SUSPENDED),
        FleetStatus::RenewalDue => Some(SCHEDULE_TERMINATION_RENEWAL),
        FleetStatus::Draft | FleetStatus::Ended => None,
    }
}

/// The fleet contract in the kernel's registry, so the approvals can run the
/// `_approved` transitions after the approval.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<FleetContract>(FLEET_CONTRACT_SUBJECT_TYPE)
        .action(TERMINATE, TERMINATE_ACTION)
        .action(SCHEDULE_TERMINATION, TERMINATE_ACTION)
        .action(SCHEDULE_TERMINATION_SUSPENDED, TERMINATE_ACTION)
        .action(SCHEDULE_TERMINATION_RENEWAL, TERMINATE_ACTION)
        .build()
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<FleetContract>();
    terminate_policy().register();
    approvals::register_subject_link(FLEET_CONTRACT_SUBJECT_TYPE, |id| {
        format!("/auto/fleet/{id}")
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = FleetContract::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("active", Some("success"), false),
                ("suspended", Some("warning"), false),
                ("renewal_due", Some("warning"), false),
                ("ended", Some("neutral"), true),
            ]
        );
        let in_force = vec!["active", "suspended", "renewal_due"];
        let expected: Vec<(&str, Vec<&str>, &str)> = vec![
            (ACTIVATE, vec!["draft"], "active"),
            (SUSPEND, vec!["active", "renewal_due"], "suspended"),
            (
                SUSPEND_BY_DECISION,
                vec!["active", "renewal_due"],
                "suspended",
            ),
            (RESUME, vec!["suspended"], "active"),
            (FLAG_RENEWAL, vec!["active"], "renewal_due"),
            (RENEW, vec!["renewal_due"], "active"),
            (END, in_force.clone(), "ended"),
            (TERMINATE, in_force.clone(), "ended"),
            (TERMINATE_APPROVED, in_force.clone(), "ended"),
            (SCHEDULE_TERMINATION, vec!["active"], "active"),
            (SCHEDULE_TERMINATION_APPROVED, vec!["active"], "active"),
            (
                SCHEDULE_TERMINATION_SUSPENDED,
                vec!["suspended"],
                "suspended",
            ),
            (
                SCHEDULE_TERMINATION_SUSPENDED_APPROVED,
                vec!["suspended"],
                "suspended",
            ),
            (
                SCHEDULE_TERMINATION_RENEWAL,
                vec!["renewal_due"],
                "renewal_due",
            ),
            (
                SCHEDULE_TERMINATION_RENEWAL_APPROVED,
                vec!["renewal_due"],
                "renewal_due",
            ),
            (TERMINATE_SYSTEM, in_force.clone(), "ended"),
            (TERMINATE_BY_DECISION, in_force, "ended"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        let system = [
            FLAG_RENEWAL,
            END,
            TERMINATE_SYSTEM,
            SUSPEND_BY_DECISION,
            TERMINATE_BY_DECISION,
            TERMINATE_APPROVED,
            SCHEDULE_TERMINATION_APPROVED,
            SCHEDULE_TERMINATION_SUSPENDED_APPROVED,
            SCHEDULE_TERMINATION_RENEWAL_APPROVED,
        ];
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = FleetContract::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            guards(ACTIVATE),
            [
                "vehicles_set",
                "rates_set",
                "schedule_set",
                "signed_contract"
            ]
        );
        assert!(guards(SUSPEND).is_empty());
        assert!(guards(RESUME).is_empty());
        assert_eq!(guards(FLAG_RENEWAL), ["end_within_window"]);
        assert_eq!(guards(RENEW), ["new_end_date", "rates_set"]);
        assert_eq!(guards(END), ["end_date_passed"]);
        for name in [
            TERMINATE,
            TERMINATE_APPROVED,
            SCHEDULE_TERMINATION,
            SCHEDULE_TERMINATION_APPROVED,
            SCHEDULE_TERMINATION_SUSPENDED,
            SCHEDULE_TERMINATION_RENEWAL,
        ] {
            assert_eq!(
                guards(name),
                ["termination_dates", "not_scheduled", "owner_approval"],
                "{name}"
            );
        }
        for name in [TERMINATE_SYSTEM, TERMINATE_BY_DECISION] {
            assert_eq!(
                guards(name),
                ["termination_dates", "owner_approval"],
                "{name}"
            );
        }
        assert!(guards(SUSPEND_BY_DECISION).is_empty());
        for name in [
            SUSPEND,
            SUSPEND_BY_DECISION,
            TERMINATE,
            TERMINATE_SYSTEM,
            TERMINATE_BY_DECISION,
            SCHEDULE_TERMINATION,
        ] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }

    #[test]
    fn a_termination_is_one_approval_action_whichever_request_is_made() {
        let sub = subject_machine();
        for name in [
            TERMINATE,
            SCHEDULE_TERMINATION,
            SCHEDULE_TERMINATION_SUSPENDED,
            SCHEDULE_TERMINATION_RENEWAL,
        ] {
            assert_eq!(sub.action_key(name), TERMINATE_ACTION, "{name}");
        }
        assert_eq!(
            schedule_request(FleetStatus::Suspended),
            Some(SCHEDULE_TERMINATION_SUSPENDED)
        );
        assert_eq!(schedule_request(FleetStatus::Ended), None);
    }
}
