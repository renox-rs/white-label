//! The Deposit state machine (wl-payments §4).
//!
//! ```text
//! [*] --> held --start_settlement--> settlement_pending --refund--> refunded
//! settlement_pending --forfeit--> forfeited   (later)
//! held --transfer--> transferred              (later)
//! ```
//!
//! All five states are declared with their tones. This part builds
//! `start_settlement` (a user with `deposits.manage`) and
//! `start_settlement_by_system` (the source agreement ended), and `refund`
//! (`deposits.refund`): in its transaction a Refund is requested with the
//! deposit as source for the held amount, and linked to the deposit. The
//! deposit stays `refunded` whatever happens to that refund. `forfeit`,
//! `transfer` and `adjust` come with the issues that follow, into this
//! `machine()`.

use renox::prelude::*;
#[cfg(test)]
use renox::state_machine::TransitionError;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::PaymentMethod;

use crate::deposit::{Deposit, DepositStatus, method_problem};
use crate::permissions;
use crate::refund::request_for_deposit_in;

/// The `start_settlement` transition's name.
pub const START_SETTLEMENT: &str = "start_settlement";
/// The `start_settlement_by_system` transition's name.
pub const START_SETTLEMENT_BY_SYSTEM: &str = "start_settlement_by_system";
/// The `refund` transition's name.
pub const REFUND: &str = "refund";

/// What a `refund` run carries to its guards and effect, gathered by
/// [`crate::deposit::refund`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RefundDepositInput {
    /// How the deposit is paid back; `None` is refused.
    pub method: Option<PaymentMethod>,
}

fn refund_input(ctx: &TransitionCtx) -> RefundDepositInput {
    ctx.input_as::<RefundDepositInput>().unwrap_or_default()
}

impl StateMachine for Deposit {
    type State = DepositStatus;
    const COLUMN: renox::db::Col<Deposit, DepositStatus> = Deposit::STATUS;

    fn machine() -> Machine<Self> {
        use DepositStatus::{Forfeited, Held, Refunded, SettlementPending, Transferred};
        let machine = Machine::new()
            .initial([Held])
            .state(Held)
            .tone("info")
            .state(SettlementPending)
            .tone("warning")
            .state(Refunded)
            .tone("success")
            .terminal()
            .state(Forfeited)
            .tone("neutral")
            .terminal()
            .state(Transferred)
            .tone("neutral")
            .terminal();
        transitions().into_iter().fold(machine, Machine::transition)
    }
}

fn transitions() -> Vec<Transition<Deposit>> {
    vec![start_settlement(), start_settlement_by_system(), refund()]
}

fn start_settlement() -> Transition<Deposit> {
    Transition::new(START_SETTLEMENT)
        .label("Start settlement")
        .tone("primary")
        .permission(permissions::DEPOSITS_MANAGE)
        .from([DepositStatus::Held])
        .to(DepositStatus::SettlementPending)
}

fn start_settlement_by_system() -> Transition<Deposit> {
    Transition::new(START_SETTLEMENT_BY_SYSTEM)
        .label("Start settlement")
        .tone("primary")
        .system()
        .from([DepositStatus::Held])
        .to(DepositStatus::SettlementPending)
}

fn refund() -> Transition<Deposit> {
    Transition::new(REFUND)
        .label("Refund")
        .tone("primary")
        .permission(permissions::DEPOSITS_REFUND)
        .from([DepositStatus::SettlementPending])
        .to(DepositStatus::Refunded)
        .guard("method_given", |_deposit: &Deposit, ctx| {
            let problem = method_problem(refund_input(ctx).method);
            Box::pin(async move { problem.map_or(Ok(()), |text| Err(text.to_owned())) })
        })
        .guard("holds_money", |deposit: &Deposit, _ctx| {
            let held = deposit.held_amount;
            Box::pin(async move {
                if held > 0 {
                    Ok(())
                } else {
                    Err("The deposit holds nothing to refund.".to_owned())
                }
            })
        })
        .guard("not_refunded_yet", |deposit: &Deposit, _ctx| {
            let linked = deposit.refund_id.is_some();
            Box::pin(async move {
                if linked {
                    Err("The deposit already has a refund.".to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|deposit: &Deposit, ctx, tx| {
            let deposit = deposit.clone();
            let method = refund_input(ctx).method;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let method = method.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a deposit refund without a method"))
                })?;
                // Requested like any refund: it waits for an approval.
                let refund = request_for_deposit_in(&mut *tx, &deposit, method, by).await?;
                // Not the status column, so outside the machine's guard.
                Deposit::query()
                    .where_(Deposit::ID.eq(deposit.id))
                    .update(
                        &mut *tx,
                        &[
                            ("refund_method", &Some(method.as_str().to_owned())),
                            ("refund_id", &Some(refund.id)),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_five_states_are_declared_and_a_deposit_starts_held() {
        let machine = Deposit::machine();
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in [
            "held",
            "settlement_pending",
            "refunded",
            "forfeited",
            "transferred",
        ] {
            assert!(names.contains(&state), "{state}");
        }
        assert_eq!(machine.initial_states(), ["held"]);
        for terminal in ["refunded", "forfeited", "transferred"] {
            let state = machine.states().iter().find(|s| s.name == terminal);
            assert!(state.is_some_and(|s| s.terminal), "{terminal}");
        }
    }

    #[test]
    fn the_states_have_tones() {
        let machine = Deposit::machine();
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }

    #[test]
    fn each_transition_leaves_only_its_states() {
        let machine = Deposit::machine();
        for name in [START_SETTLEMENT, START_SETTLEMENT_BY_SYSTEM] {
            let found = machine.find(name).expect("transition");
            assert!(found.check_from(DepositStatus::Held).is_ok(), "{name}");
            for other in [
                DepositStatus::SettlementPending,
                DepositStatus::Refunded,
                DepositStatus::Forfeited,
            ] {
                assert!(found.check_from(other).is_err(), "{name}");
            }
        }
        let found = machine.find(REFUND).expect("transition");
        assert!(found.check_from(DepositStatus::SettlementPending).is_ok());
        assert!(found.check_from(DepositStatus::Held).is_err());
        assert!(found.check_from(DepositStatus::Refunded).is_err());
    }

    #[renox::test]
    async fn refund_needs_a_method_money_held_and_no_refund_yet() {
        let machine = Deposit::machine();
        let found = machine.find(REFUND).expect("transition");
        let with = |method| {
            TransitionCtx::new(None).with_input(
                renox::serde_json::to_value(RefundDepositInput { method }).expect("json"),
            )
        };
        let held = Deposit {
            held_amount: 800_000,
            ..Default::default()
        };
        let ok = with(Some(PaymentMethod::Transfer));
        assert!(found.check_guards(&held, &ok).await.is_ok());
        let refused = |deposit: Deposit, ctx: TransitionCtx| async move {
            matches!(
                found.check_guards(&deposit, &ctx).await,
                Err(TransitionError::Guard(_))
            )
        };
        assert!(refused(held.clone(), with(None)).await);
        assert!(refused(held.clone(), with(Some(PaymentMethod::Cash))).await);
        assert!(refused(Deposit::default(), with(Some(PaymentMethod::Cheque))).await);
        let linked = Deposit {
            refund_id: Some(4),
            ..held
        };
        assert!(refused(linked, with(Some(PaymentMethod::Transfer))).await);
    }

    #[test]
    fn a_refund_input_without_a_method_is_refused() {
        let ctx = TransitionCtx::new(None);
        assert!(method_problem(refund_input(&ctx).method).is_some());
        let input = RefundDepositInput {
            method: Some(PaymentMethod::Transfer),
        };
        let ctx =
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"));
        assert_eq!(method_problem(refund_input(&ctx).method), None);
    }
}
