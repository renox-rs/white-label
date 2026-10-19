//! The Service contract state machine (wl-ticketing §4.6), as far as the
//! negative flows need it.
//!
//! ```text
//! [*] --> draft --activate--> active --flag_expiring (system, WRK.RUL.12)--> expiring
//!         expiring --expire (system, WRK.RUL.13)--> expired (terminal)
//!         active | expiring --terminate--> terminated (terminal)
//!         active | expiring --terminate_by_decision (system)--> terminated
//!         expiring --renew--> renewed (terminal; with the renewal issue)
//! ```
//!
//! `renew` and `discard` are not built here, so `renewed` has no way in yet.
//! `terminate_by_decision` is `terminate` run by the system for the decider
//! of a blocked vendor's review (`PTY.ACL.03`); the history names who decided.
//! `ServiceContract` is `#[model(state_machine)]`, so `save` refuses a
//! changed status.

use renox::chrono::Duration;
use renox::db::ToDbValue;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};

use crate::contract::{ContractInput, ContractStatus, ServiceContract};
use crate::permissions;
use crate::plans::MaintenancePlans;

/// `activate`: the contract is in force.
pub const ACTIVATE: &str = "activate";
/// `flag_expiring`: the renewal notice window is reached.
pub const FLAG_EXPIRING: &str = "flag_expiring";
/// `expire`: the end date passed without renewal.
pub const EXPIRE: &str = "expire";
/// `terminate`: ended early.
pub const TERMINATE: &str = "terminate";
/// `terminate`, run by the system on a decider's choice.
pub const TERMINATE_BY_DECISION: &str = "terminate_by_decision";

/// The run's input, if it is a [`ContractInput`].
fn input(ctx: &TransitionCtx) -> Option<ContractInput> {
    ctx.input_as::<ContractInput>().ok()
}

/// Adds the guard that refuses a run without a [`ContractInput`].
fn with_input(transition: Transition<ServiceContract>) -> Transition<ServiceContract> {
    transition.guard("input", |_contract: &ServiceContract, ctx| {
        let present = input(ctx).is_some();
        Box::pin(async move {
            if present {
                Ok(())
            } else {
                Err("The action was not prepared by the contract service.".to_owned())
            }
        })
    })
}

/// The guards and effect `terminate` and `terminate_by_decision` share: a
/// termination date not before the start date, stored with the reason.
fn terminating(transition: Transition<ServiceContract>) -> Transition<ServiceContract> {
    with_input(
        transition
            .from([ContractStatus::Active, ContractStatus::Expiring])
            .to(ContractStatus::Terminated)
            .reason_required(),
    )
    .guard("date", |contract: &ServiceContract, ctx| {
        let verdict = match input(ctx).and_then(|i| i.termination_date) {
            Some(date) if date >= contract.start_date => Ok(()),
            Some(_) => Err("The termination date is before the start date.".to_owned()),
            None => Err("A termination needs a termination date.".to_owned()),
        };
        Box::pin(async move { verdict })
    })
    .effect(|contract: &ServiceContract, ctx, tx| {
        let id = contract.id;
        let date = input(ctx).and_then(|i| i.termination_date);
        let reason = ctx.reason().map(|r| r.trim().to_owned());
        Box::pin(async move {
            // Not the status column, so outside the machine's guard.
            ServiceContract::query()
                .where_(ServiceContract::ID.eq(id))
                .update(
                    &mut *tx,
                    &[("termination_date", &date), ("termination_reason", &reason)],
                )
                .await?;
            Ok(())
        })
    })
}

/// `activate`: draft → active, by `service_contracts.manage`, for a contract
/// that covers at least one subject; creates one plan per covered subject
/// and category.
fn activate_transition() -> Transition<ServiceContract> {
    with_input(
        Transition::new(ACTIVATE)
            .label("Activate")
            .tone("primary")
            .permission(permissions::SERVICE_CONTRACTS_MANAGE)
            .from([ContractStatus::Draft])
            .to(ContractStatus::Active),
    )
    .guard("covered", |_contract: &ServiceContract, ctx| {
        let covered = input(ctx).is_some_and(|i| i.covered > 0);
        Box::pin(async move {
            if covered {
                Ok(())
            } else {
                Err("A contract needs at least one covered subject.".to_owned())
            }
        })
    })
    .guard("dates", |contract: &ServiceContract, _ctx| {
        let ordered = contract.end_date >= contract.start_date;
        Box::pin(async move {
            if ordered {
                Ok(())
            } else {
                Err("The end date is before the start date.".to_owned())
            }
        })
    })
    .effect(|contract: &ServiceContract, ctx, tx| {
        let id = contract.id;
        let input = input(ctx);
        Box::pin(async move {
            let input = input.ok_or_else(|| {
                Error::from(renox::anyhow::anyhow!("an activate without its input"))
            })?;
            let compliance = input.compliance;
            let values: Vec<(&str, &(dyn ToDbValue + Sync))> =
                vec![("compliance_contract", &compliance)];
            ServiceContract::query()
                .where_(ServiceContract::ID.eq(id))
                .update(&mut *tx, &values)
                .await?;
            for seed in &input.plans {
                MaintenancePlans::create_in(&mut *tx, id, seed).await?;
            }
            Ok(())
        })
    })
}

/// `flag_expiring`: active → expiring, by the system, once the end date minus
/// the renewal notice days is reached.
fn flag_expiring_transition() -> Transition<ServiceContract> {
    with_input(
        Transition::new(FLAG_EXPIRING)
            .label("Flag as expiring")
            .tone("warning")
            .from([ContractStatus::Active])
            .to(ContractStatus::Expiring)
            .system(),
    )
    .guard("window", |contract: &ServiceContract, ctx| {
        let reached = input(ctx).is_some_and(|i| {
            i.today >= contract.end_date - Duration::days(contract.renewal_notice_days)
        });
        Box::pin(async move {
            if reached {
                Ok(())
            } else {
                Err("The renewal notice window is not reached.".to_owned())
            }
        })
    })
}

/// `expire`: expiring → expired, by the system, once the end date has passed.
fn expire_transition() -> Transition<ServiceContract> {
    with_input(
        Transition::new(EXPIRE)
            .label("Expire")
            .tone("danger")
            .from([ContractStatus::Expiring])
            .to(ContractStatus::Expired)
            .system(),
    )
    .guard("end_passed", |contract: &ServiceContract, ctx| {
        let passed = input(ctx).is_some_and(|i| i.today > contract.end_date);
        Box::pin(async move {
            if passed {
                Ok(())
            } else {
                Err("The end date has not passed.".to_owned())
            }
        })
    })
}

/// `terminate`: active or expiring → terminated, by `service_contracts.manage`,
/// with a reason and a termination date.
fn terminate_transition() -> Transition<ServiceContract> {
    terminating(
        Transition::new(TERMINATE)
            .label("Terminate")
            .tone("danger")
            .permission(permissions::SERVICE_CONTRACTS_MANAGE),
    )
}

/// `terminate`, run by the system on a decider's choice.
fn terminate_by_decision_transition() -> Transition<ServiceContract> {
    terminating(
        Transition::new(TERMINATE_BY_DECISION)
            .label("Terminate (decision)")
            .system(),
    )
}

impl StateMachine for ServiceContract {
    type State = ContractStatus;
    const COLUMN: renox::db::Col<ServiceContract, ContractStatus> = ServiceContract::STATUS;

    fn machine() -> Machine<Self> {
        use ContractStatus::{Active, Draft, Expired, Expiring, Renewed, Terminated};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Expiring)
            .tone("warning")
            .state(Renewed)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
            .state(Terminated)
            .tone("neutral")
            .terminal()
            .transition(activate_transition())
            .transition(flag_expiring_transition())
            .transition(expire_transition())
            .transition(terminate_transition())
            .transition(terminate_by_decision_transition())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_is_declared_with_its_tone() {
        let machine = ServiceContract::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("active", Some("success"), false),
                ("expiring", Some("warning"), false),
                ("renewed", Some("success"), true),
                ("expired", Some("neutral"), true),
                ("terminated", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.initial_states(), ["draft"]);
    }

    #[test]
    fn every_transition_exists() {
        let machine = ServiceContract::machine();
        let expected = [
            ("activate", vec!["draft"], "active", false),
            ("flag_expiring", vec!["active"], "expiring", true),
            ("expire", vec!["expiring"], "expired", true),
            ("terminate", vec!["active", "expiring"], "terminated", false),
            (
                "terminate_by_decision",
                vec!["active", "expiring"],
                "terminated",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn terminating_needs_a_reason_and_the_others_do_not() {
        let machine = ServiceContract::machine();
        for transition in machine.transitions() {
            let terminates = transition.name().starts_with("terminate");
            assert_eq!(transition.needs_reason(), terminates, "{}", transition.name());
        }
    }

    #[test]
    fn the_guards_are_those_of_the_plan() {
        let machine = ServiceContract::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .expect("transition")
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards("activate"), ["input", "covered", "dates"]);
        assert_eq!(guards("flag_expiring"), ["input", "window"]);
        assert_eq!(guards("expire"), ["input", "end_passed"]);
        assert_eq!(guards("terminate"), ["input", "date"]);
        assert_eq!(guards("terminate_by_decision"), ["input", "date"]);
    }
}
