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
//! | [`TERMINATE`] / [`TERMINATE_SYSTEM`] | active, suspended, renewal_due | ended | `fleet_contracts.manage` / system (daily job) | a reason; `termination_dates`, `owner_approval` (slot) |
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
//!   told, open job cards finished - is added by BL-AUT-13 (`end`) and
//!   BL-AUT-12 (`terminate`) in the service, after the commit. `terminate` is
//!   declared twice, for a user and for the system (the daily job runs it on
//!   the first day after the effective date); the guard slot `owner_approval`
//!   passes for now and is where BL-AUT-12 adds the owner's approval
//!   (`PLT.APR.01`, not the requester).
//! - A suspended contract keeps its vehicles and its job cards; new bookings
//!   are not requested while it is suspended ([`crate::fleet_services`]).

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::db::set_fleet_contract;
use crate::fleet_contract::{FleetContract, FleetStatus};
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
/// `active, suspended, renewal_due → ended`: terminated early, by the daily
/// job on the first day after the effective date.
pub const TERMINATE_SYSTEM: &str = "terminate_system";

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
}

fn input_of(ctx: &TransitionCtx) -> FleetInput {
    ctx.input_as::<FleetInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// A guard slot: it names a rule an issue adds and passes for now.
fn slot(transition: Transition<FleetContract>, name: &'static str) -> Transition<FleetContract> {
    transition.guard(name, |_c: &FleetContract, _ctx| {
        Box::pin(async { Ok::<(), String>(()) })
    })
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
            .transition(suspend())
            .transition(resume())
            .transition(flag_renewal())
            .transition(renew())
            .transition(end())
            .transition(terminate(TERMINATE, false))
            .transition(terminate(TERMINATE_SYSTEM, true))
    }
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
/// open job cards finish.
fn suspend() -> Transition<FleetContract> {
    Transition::new(SUSPEND)
        .label("Suspend")
        .permission(permissions::FLEET_CONTRACTS_MANAGE)
        .from([FleetStatus::Active, FleetStatus::RenewalDue])
        .to(FleetStatus::Suspended)
        .reason_required()
        .effect(|c: &FleetContract, ctx, tx| {
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

/// `terminate`: the contract ends early with a reason, a notice date and an
/// effective date (the last day of service).
fn terminate(name: &'static str, system: bool) -> Transition<FleetContract> {
    let transition = Transition::new(name)
        .label("Terminate")
        .from([
            FleetStatus::Active,
            FleetStatus::Suspended,
            FleetStatus::RenewalDue,
        ])
        .to(FleetStatus::Ended)
        .reason_required();
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::FLEET_CONTRACTS_MANAGE)
    };
    let transition = with_termination_dates(transition);
    slot(transition, "owner_approval").effect(|c: &FleetContract, ctx, tx| {
        let id = c.id;
        let input = input_of(ctx);
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            set_fleet_contract(
                tx,
                id,
                &[
                    ("termination_reason", &reason),
                    ("notice_date", &input.notice_date),
                    ("effective_date", &input.effective_date),
                    ("termination_scheduled", &false),
                    ("termination_requested_by", &input.by),
                    ("ended_at", &Some(now())),
                ],
            )
            .await
        })
    })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<FleetContract>();
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
            (RESUME, vec!["suspended"], "active"),
            (FLAG_RENEWAL, vec!["active"], "renewal_due"),
            (RENEW, vec!["renewal_due"], "active"),
            (END, in_force.clone(), "ended"),
            (TERMINATE, in_force.clone(), "ended"),
            (TERMINATE_SYSTEM, in_force, "ended"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        let system = [FLAG_RENEWAL, END, TERMINATE_SYSTEM];
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
        assert_eq!(guards(TERMINATE), ["termination_dates", "owner_approval"]);
        assert_eq!(
            guards(TERMINATE_SYSTEM),
            ["termination_dates", "owner_approval"]
        );
        for name in [SUSPEND, TERMINATE, TERMINATE_SYSTEM] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }
}
