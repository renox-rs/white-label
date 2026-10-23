//! The supplier warranty claim machine (08 §7.1 *Supplier warranty claim*;
//! `AUT.WAR.02`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | `raise` | — | raised | `supplier_claims.manage` | the service: the warranty ticket closed as *accepted*; the supplier warranty applies |
//! | [`RECOVER`] / [`RECOVER_SYSTEM`] | raised | recovered | `supplier_claims.manage` / system (`SupplierReturnCredited`) | `credit_or_replacement` |
//! | [`WRITE_OFF`] | raised | written_off | `supplier_claims.manage` | a reason; `branch_manager_approval` (slot) |
//!
//! Decisions:
//!
//! - `raise` is how a claim comes to be (the initial state is `raised`): the
//!   service ([`crate::supplier_claims::raise`]) checks what a record that does
//!   not exist yet cannot be guarded for, then inserts the claim in `raised`
//!   with the history row `raise`.
//! - `recover` needs the supplier's credit note (a supplier debit note settled
//!   in wl-invoicing) or a replacement received; a manual credit with the
//!   reason *supplier refused* does not recover (the claim stays `raised` and
//!   BL-AUT-11 gives the purchaser a write-off task).
//! - The guard slot `branch_manager_approval` passes for now: BL-AUT-11 adds
//!   the approval of a branch manager other than the requester (`PLT.APR.01`).

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::db::set_supplier_claim;
use crate::permissions;
use crate::supplier_claim::{SupplierClaim, SupplierClaimStatus};
use crate::texts::english;

/// `raised → recovered`: the supplier credited or replaced the part.
pub const RECOVER: &str = "recover";
/// `raised → recovered`: the Return to supplier was credited
/// (`SupplierReturnCredited`).
pub const RECOVER_SYSTEM: &str = "recover_system";
/// `raised → written_off`: the supplier refused and the cost is absorbed.
pub const WRITE_OFF: &str = "write_off";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierInput {
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// How the claim was recovered: `credit_note`, `replacement`,
    /// `refund_received` or `manual`.
    #[serde(default)]
    pub basis: Option<String>,
    /// The supplier's credit note, or the receipt of the replacement.
    #[serde(default)]
    pub credit_reference: Option<String>,
    /// A replacement part was received against the claim.
    #[serde(default)]
    pub replacement_received: Option<bool>,
    /// The branch manager who approved a write-off.
    #[serde(default)]
    pub approved_by: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> SupplierInput {
    ctx.input_as::<SupplierInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// A guard slot: it names a rule an issue adds and passes for now.
fn slot(transition: Transition<SupplierClaim>, name: &'static str) -> Transition<SupplierClaim> {
    transition.guard(name, |_c: &SupplierClaim, _ctx| {
        Box::pin(async { Ok::<(), String>(()) })
    })
}

impl StateMachine for SupplierClaim {
    type State = SupplierClaimStatus;
    const COLUMN: renox::db::Col<SupplierClaim, SupplierClaimStatus> = SupplierClaim::STATUS;

    fn machine() -> Machine<Self> {
        use SupplierClaimStatus::{Raised, Recovered, WrittenOff};
        Machine::new()
            .initial([Raised])
            .state(Raised)
            .tone("warning")
            .state(Recovered)
            .tone("success")
            .terminal()
            .state(WrittenOff)
            .tone("neutral")
            .terminal()
            .transition(recover(RECOVER, false))
            .transition(recover(RECOVER_SYSTEM, true))
            .transition(write_off())
    }
}

/// `recover`: the supplier credited the claim (a settled debit note) or sent a
/// replacement.
fn recover(name: &'static str, system: bool) -> Transition<SupplierClaim> {
    let transition = Transition::new(name)
        .label("Recover")
        .tone("primary")
        .from([SupplierClaimStatus::Raised])
        .to(SupplierClaimStatus::Recovered);
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::SUPPLIER_CLAIMS_MANAGE)
    };
    transition
        .guard("credit_or_replacement", |_c: &SupplierClaim, ctx| {
            let input = input_of(ctx);
            let credited = input
                .credit_reference
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty());
            let ok = credited || input.replacement_received == Some(true);
            Box::pin(async move { check(ok, "auto.supplier.problem.no_credit") })
        })
        .effect(|c: &SupplierClaim, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            Box::pin(async move {
                let basis = input.basis.clone().or_else(|| {
                    Some(
                        if input.replacement_received == Some(true) {
                            "replacement"
                        } else {
                            "credit_note"
                        }
                        .to_owned(),
                    )
                });
                set_supplier_claim(
                    tx,
                    id,
                    &[
                        ("recovered_at", &Some(now())),
                        ("recovery_basis", &basis),
                        ("recovery_reference", &input.credit_reference),
                    ],
                )
                .await
            })
        })
}

/// `write_off`: the supplier refused; the cost is booked as warranty expense.
fn write_off() -> Transition<SupplierClaim> {
    let transition = Transition::new(WRITE_OFF)
        .label("Write off")
        .permission(permissions::SUPPLIER_CLAIMS_MANAGE)
        .from([SupplierClaimStatus::Raised])
        .to(SupplierClaimStatus::WrittenOff)
        .reason_required();
    slot(transition, "branch_manager_approval").effect(|c: &SupplierClaim, ctx, tx| {
        let id = c.id;
        let reason = ctx.reason().map(str::to_owned);
        let approved_by = input_of(ctx).approved_by;
        Box::pin(async move {
            set_supplier_claim(
                tx,
                id,
                &[
                    ("written_off_at", &Some(now())),
                    ("write_off_reason", &reason),
                    ("write_off_approved_by", &approved_by),
                ],
            )
            .await
        })
    })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<SupplierClaim>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = SupplierClaim::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("raised", Some("warning"), false),
                ("recovered", Some("success"), true),
                ("written_off", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.transitions().len(), 3);
        for (name, to, system) in [
            (RECOVER, "recovered", false),
            (RECOVER_SYSTEM, "recovered", true),
            (WRITE_OFF, "written_off", false),
        ] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), ["raised"], "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slot_are_declared() {
        let machine = SupplierClaim::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards(RECOVER), ["credit_or_replacement"]);
        assert_eq!(guards(RECOVER_SYSTEM), ["credit_or_replacement"]);
        assert_eq!(guards(WRITE_OFF), ["branch_manager_approval"]);
        assert!(machine.find(WRITE_OFF).unwrap().needs_reason());
    }
}
