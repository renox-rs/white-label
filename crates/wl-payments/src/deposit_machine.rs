//! The Deposit state machine (wl-payments §4).
//!
//! ```text
//! [*] --> held --start_settlement--> settlement_pending --refund--> refunded
//! settlement_pending --forfeit--> forfeited
//! held --transfer--> transferred
//! held --adjust--> held
//! ```
//!
//! All five states are declared with their tones. This part builds
//! `start_settlement` (a user with `deposits.manage`) and
//! `start_settlement_by_system` (the source agreement ended), and `refund`
//! (`deposits.refund`): money out to the party, so a blocked party's refund
//! waits for an approved exception (`PTY.ACL.03`, `refund_approved`,
//! [`crate::deposit::refund_or_ask`]): in its transaction a Refund is requested with the
//! deposit as source for the held amount, and linked to the deposit. The
//! deposit stays `refunded` whatever happens to that refund. `forfeit`
//! (`deposits.refund`, with a reason and a second user's approval) keeps the
//! deposit in full. `refund` and `forfeit` wait for the deductions: no
//! deductions dispute may be open, and with the setting on the dispute window
//! of the latest published version must have passed
//! ([`crate::deposit_deductions`]). `transfer` (`deposits.manage`, a held
//! deposit, the new holder's receipt) hands the deposit to another holder with
//! the source agreement. `adjust` and `adjust_by_system`
//! ([`crate::deposit_adjust`]) change the amount of a held deposit when its
//! agreement changes; the deposit stays `held`.

use renox::prelude::*;
#[cfg(test)]
use renox::state_machine::TransitionError;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::payments::PaymentMethod;

use crate::deposit::{Deposit, DepositStatus, method_problem};
use crate::deposit_deductions::{DeductionGate, gate_problem};
use crate::permissions;
use crate::refund::request_for_deposit_in;

/// The `start_settlement` transition's name.
pub const START_SETTLEMENT: &str = "start_settlement";
/// The `start_settlement_by_system` transition's name.
pub const START_SETTLEMENT_BY_SYSTEM: &str = "start_settlement_by_system";
/// The `refund` transition's name.
pub const REFUND: &str = "refund";
/// The `refund_approved` transition's name: the kernel runs it when an
/// exception for a blocked party was approved (`PTY.ACL.03`).
pub const REFUND_APPROVED: &str = "refund_approved";
/// The `forfeit` transition's name.
pub const FORFEIT: &str = "forfeit";
/// The `transfer` transition's name.
pub const TRANSFER: &str = "transfer";

/// What a `refund` run carries to its guards and effect, gathered by
/// [`crate::deposit::refund`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RefundDepositInput {
    /// How the deposit is paid back; `None` is refused.
    pub method: Option<PaymentMethod>,
    /// What the deductions say about running it now; `None` (a run the
    /// deposit service did not prepare) is refused.
    pub gate: Option<DeductionGate>,
    /// Whether a part refund of an adjustment is not settled yet: the refund
    /// at settlement waits for it.
    pub part_refund_open: bool,
}

/// What a `forfeit` run carries to its guards and effect, gathered by
/// [`crate::deposit::forfeit`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ForfeitInput {
    /// Why the deposit is kept, trimmed.
    pub reason: String,
    /// The second user who approved it.
    pub approver: Option<i64>,
    /// Whether that user exists, holds `deposits.refund` and is not the one
    /// who forfeits.
    pub approver_ok: bool,
    /// What the deductions say about running it now.
    pub gate: Option<DeductionGate>,
}

/// What a `transfer` run carries to its guards, gathered by
/// [`crate::deposit::transfer`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TransferInput {
    /// The reference of the new holder's receipt, trimmed.
    pub receipt: String,
    /// The new holder, a party.
    pub new_holder: i64,
    /// Whether that party exists and is not the deposit's own party.
    pub holder_ok: bool,
    /// Whether a part refund of an adjustment is not settled yet.
    pub part_refund_open: bool,
}

fn transfer_input(ctx: &TransitionCtx) -> TransferInput {
    ctx.input_as::<TransferInput>().unwrap_or_default()
}

/// Why the deposit cannot be handed over with `input`, if it cannot.
pub(crate) fn transfer_problem(deposit: &Deposit, input: &TransferInput) -> Option<String> {
    if input.receipt.trim().is_empty() {
        return Some("The new holder's receipt must be attached.".to_owned());
    }
    if !input.holder_ok {
        return Some("The new holder must be another party that exists.".to_owned());
    }
    if deposit.held_amount <= 0 {
        return Some("The deposit holds nothing to hand over.".to_owned());
    }
    if input.part_refund_open {
        return Some(
            "A part refund of an adjustment is not settled yet: pay or cancel it first.".to_owned(),
        );
    }
    None
}

fn forfeit_input(ctx: &TransitionCtx) -> ForfeitInput {
    ctx.input_as::<ForfeitInput>().unwrap_or_default()
}

/// Why the deposit cannot be forfeited with `input`, if it cannot.
pub(crate) fn forfeit_problem(deposit: &Deposit, input: &ForfeitInput) -> Option<String> {
    if input.reason.trim().is_empty() {
        return Some("A reason is required.".to_owned());
    }
    if input.approver.is_none() || !input.approver_ok {
        return Some(
            "The forfeiture needs the approval of another user who may refund deposits.".to_owned(),
        );
    }
    if deposit.refund_id.is_some() {
        return Some("The deposit already has a refund.".to_owned());
    }
    match &input.gate {
        Some(gate) => gate_problem(gate, true),
        None => Some("The forfeiture was not prepared by the deposit service.".to_owned()),
    }
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
    let mut all = vec![
        start_settlement(),
        start_settlement_by_system(),
        refund(),
        refund_approved(),
        forfeit(),
        transfer(),
    ];
    all.extend(crate::deposit_adjust::transitions());
    all
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

/// Why `deposit` cannot be refunded with `input`, if it cannot: a method of
/// transfer or cheque is given, the deductions allow it, no part refund of an
/// adjustment is open, the deposit holds money and has no refund yet.
pub(crate) fn refund_problem(deposit: &Deposit, input: &RefundDepositInput) -> Option<String> {
    if let Some(text) = method_problem(input.method) {
        return Some(text.to_owned());
    }
    match &input.gate {
        Some(gate) => {
            if let Some(problem) = gate_problem(gate, false) {
                return Some(problem);
            }
        }
        None => return Some("The refund was not prepared by the deposit service.".to_owned()),
    }
    if input.part_refund_open {
        return Some(
            "A part refund of an adjustment is not settled yet: pay or cancel it first.".to_owned(),
        );
    }
    if deposit.held_amount <= 0 {
        return Some("The deposit holds nothing to refund.".to_owned());
    }
    if deposit.refund_id.is_some() {
        return Some("The deposit already has a refund.".to_owned());
    }
    None
}

/// The guard and the effect `refund` and `refund_approved` share.
fn refund_rules(transition: Transition<Deposit>) -> Transition<Deposit> {
    transition
        .from([DepositStatus::SettlementPending])
        .to(DepositStatus::Refunded)
        .guard("refund_possible", |deposit: &Deposit, ctx| {
            let problem = refund_problem(deposit, &refund_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
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

fn refund() -> Transition<Deposit> {
    refund_rules(
        Transition::new(REFUND)
            .label("Refund")
            .tone("primary")
            .permission(permissions::DEPOSITS_REFUND),
    )
}

/// The refund the kernel runs once an exception for a blocked party was
/// approved: the same guards and effect, run by the system with the payload
/// of the request.
fn refund_approved() -> Transition<Deposit> {
    refund_rules(
        Transition::new(REFUND_APPROVED)
            .label("Refund (exception approved)")
            .tone("primary")
            .system(),
    )
}

fn transfer() -> Transition<Deposit> {
    Transition::new(TRANSFER)
        .label("Transfer")
        .tone("primary")
        .permission(permissions::DEPOSITS_MANAGE)
        .from([DepositStatus::Held])
        .to(DepositStatus::Transferred)
        .guard("receipt_attached", |deposit: &Deposit, ctx| {
            let problem = transfer_problem(deposit, &transfer_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
}

fn forfeit() -> Transition<Deposit> {
    Transition::new(FORFEIT)
        .label("Forfeit")
        .tone("danger")
        .permission(permissions::DEPOSITS_REFUND)
        .from([DepositStatus::SettlementPending])
        .to(DepositStatus::Forfeited)
        .guard("may_be_forfeited", |deposit: &Deposit, ctx| {
            let problem = forfeit_problem(deposit, &forfeit_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|deposit: &Deposit, ctx, tx| {
            let id = deposit.id;
            let input = forfeit_input(ctx);
            Box::pin(async move {
                // Not the status column, so outside the machine's guard.
                Deposit::query()
                    .where_(Deposit::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("forfeit_reason", &Some(input.reason.trim().to_owned())),
                            ("forfeit_approved_by", &input.approver),
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
                renox::serde_json::to_value(RefundDepositInput {
                    method,
                    gate: Some(DeductionGate::default()),
                    part_refund_open: false,
                })
                .expect("json"),
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
            gate: None,
            part_refund_open: false,
        };
        let ctx =
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"));
        assert_eq!(method_problem(refund_input(&ctx).method), None);
    }

    #[test]
    fn forfeit_leaves_settlement_pending_only() {
        let machine = Deposit::machine();
        let found = machine.find(FORFEIT).expect("transition");
        assert!(found.check_from(DepositStatus::SettlementPending).is_ok());
        for other in [
            DepositStatus::Held,
            DepositStatus::Refunded,
            DepositStatus::Forfeited,
            DepositStatus::Transferred,
        ] {
            assert!(found.check_from(other).is_err(), "{other:?}");
        }
        assert_eq!(found.to_state(), Some("forfeited"));
        assert_eq!(found.guard_names(), ["may_be_forfeited"]);
    }

    fn forfeit_ok() -> ForfeitInput {
        ForfeitInput {
            reason: "Abandoned the unit".into(),
            approver: Some(5),
            approver_ok: true,
            gate: Some(DeductionGate::default()),
        }
    }

    #[test]
    fn a_forfeiture_needs_a_reason_an_approver_and_the_deductions_to_be_ready() {
        let deposit = Deposit {
            held_amount: 800_000,
            ..Default::default()
        };
        assert_eq!(forfeit_problem(&deposit, &forfeit_ok()), None);
        let blank = ForfeitInput {
            reason: "  ".into(),
            ..forfeit_ok()
        };
        assert!(forfeit_problem(&deposit, &blank).is_some());
        let alone = ForfeitInput {
            approver: None,
            approver_ok: false,
            ..forfeit_ok()
        };
        assert!(forfeit_problem(&deposit, &alone).is_some());
        let unchecked = ForfeitInput {
            approver_ok: false,
            ..forfeit_ok()
        };
        assert!(forfeit_problem(&deposit, &unchecked).is_some());
        let unprepared = ForfeitInput {
            gate: None,
            ..forfeit_ok()
        };
        assert!(forfeit_problem(&deposit, &unprepared).is_some());
        let waiting = ForfeitInput {
            gate: Some(DeductionGate {
                open_disputes: 1,
                ..DeductionGate::default()
            }),
            ..forfeit_ok()
        };
        assert!(forfeit_problem(&deposit, &waiting).is_some());
    }

    #[test]
    fn transfer_leaves_held_only() {
        let machine = Deposit::machine();
        let found = machine.find(TRANSFER).expect("transition");
        assert!(found.check_from(DepositStatus::Held).is_ok());
        for other in [
            DepositStatus::SettlementPending,
            DepositStatus::Refunded,
            DepositStatus::Forfeited,
            DepositStatus::Transferred,
        ] {
            assert!(found.check_from(other).is_err(), "{other:?}");
        }
        assert_eq!(found.to_state(), Some("transferred"));
        assert_eq!(found.guard_names(), ["receipt_attached"]);
    }

    #[test]
    fn a_transfer_needs_the_receipt_a_new_holder_and_money_held() {
        let deposit = Deposit {
            held_amount: 1_000_000,
            ..Default::default()
        };
        let ok = TransferInput {
            receipt: "RCPT-77".into(),
            new_holder: 4,
            holder_ok: true,
            part_refund_open: false,
        };
        let waiting = TransferInput {
            part_refund_open: true,
            ..ok.clone()
        };
        assert!(
            transfer_problem(&deposit, &waiting)
                .unwrap()
                .contains("part refund")
        );
        assert_eq!(transfer_problem(&deposit, &ok), None);
        let blank = TransferInput {
            receipt: "  ".into(),
            ..ok.clone()
        };
        assert!(
            transfer_problem(&deposit, &blank)
                .unwrap()
                .contains("receipt")
        );
        let unknown = TransferInput {
            holder_ok: false,
            ..ok.clone()
        };
        assert!(transfer_problem(&deposit, &unknown).is_some());
        assert!(transfer_problem(&Deposit::default(), &ok).is_some());
    }
}
