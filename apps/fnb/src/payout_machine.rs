//! The payout statement state machine (06 F&B §7.1 *Payout statement*).
//!
//! ```text
//! [*] imported                                (import: the statement is made by its service)
//! imported --match (reconciliation.manage)--> matched
//! imported | matched --dispute (reconciliation.manage)--> disputed
//! disputed --resolve (reconciliation.manage)--> matched
//! matched --settle (system: the bank receipt was allocated)--> settled
//! imported | matched | disputed --void (reconciliation.manage; approval when matched or disputed)--> voided
//! ```
//!
//! `void` and `void_approved` come from the kernel's void pattern
//! (`PLT.ACL.09`, BL-KRN-03): the statement keeps its number, the receivable
//! booked on `match` is the posted effect that asks for an approval, and the
//! sales returns of the refund lines are taken back once the void committed
//! ([`crate::payouts::on_voided`]).
//!
//! Renox guards get no `AppState`, so the services work out what a guard
//! needs and pass it in a [`PayoutInput`].

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::machines::{BoxFuture, SubjectMachine};
use wl_kernel::voids::{self, Effect, EffectKind, Reverse, Voidable};

use crate::changes::ExpectedRefund;
use crate::order::FnbOrder;
use crate::payout::{PayoutLine, PayoutStatement, PayoutStatus, SUBJECT_TYPE};
use crate::permissions;
use crate::texts::english;

/// `match`: imported → matched.
pub const MATCH: &str = "match";
/// `match`, run by the system (the demo company, a scheduled reconciliation).
pub const MATCH_SYSTEM: &str = "match_system";
/// `dispute`: imported, matched → disputed.
pub const DISPUTE: &str = "dispute";
/// `resolve`: disputed → matched.
pub const RESOLVE: &str = "resolve";
/// `settle`: matched → settled, by the system.
pub const SETTLE: &str = "settle";
/// The approval action of the void.
pub const VOID_ACTION: &str = "fnb.payout_void";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutInput {
    /// Lines with no order that are not explained (`match`).
    #[serde(default)]
    pub unmatched: i64,
    /// Refund lines with no responsible party yet (`match`).
    #[serde(default)]
    pub refunds_without_party: i64,
    /// The lines the dispute is about (`dispute`).
    #[serde(default)]
    pub disputed_lines: Vec<i64>,
    /// What the adjustments of the answer add to the receivable, in fils
    /// (`resolve`): a refund the aggregator takes back adds its amount.
    #[serde(default)]
    pub adjustment: i64,
}

fn input_of(ctx: &TransitionCtx) -> PayoutInput {
    ctx.input_as::<PayoutInput>().unwrap_or_default()
}

/// Sets the columns of statement `id` other than the status, and stamps
/// `updated_at`.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = renox::db::now();
    all.push(("updated_at", &stamp));
    PayoutStatement::query()
        .where_(PayoutStatement::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

fn matching(name: &'static str, by_user: bool) -> Transition<PayoutStatement> {
    let base = if by_user {
        Transition::new(name)
            .label("Match")
            .tone("primary")
            .permission(permissions::RECONCILIATION_MANAGE)
    } else {
        Transition::new(name).label("Match").system()
    };
    base.from([PayoutStatus::Imported])
        .to(PayoutStatus::Matched)
        .guard("every_line_matched_or_explained", |_s: &PayoutStatement, ctx| {
            let unmatched = input_of(ctx).unmatched;
            Box::pin(async move {
                if unmatched > 0 {
                    Err(english("fnb.payout.problem.unmatched", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("every_refund_has_a_party", |_s: &PayoutStatement, ctx| {
            let missing = input_of(ctx).refunds_without_party;
            Box::pin(async move {
                if missing > 0 {
                    Err(english("fnb.payout.problem.party_missing", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|s: &PayoutStatement, ctx, tx| {
            let id = s.id;
            let net = s.net_total;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(renox::db::now());
                set(
                    tx,
                    id,
                    &[
                        ("matched_at", &at),
                        ("matched_by", &by),
                        ("receivable_amount", &net),
                        ("receivable_booked_at", &at),
                    ],
                )
                .await
            })
        })
}

fn dispute() -> Transition<PayoutStatement> {
    Transition::new(DISPUTE)
        .label("Dispute")
        .tone("warning")
        .permission(permissions::RECONCILIATION_MANAGE)
        .from([PayoutStatus::Imported, PayoutStatus::Matched])
        .to(PayoutStatus::Disputed)
        .reason_required()
        .guard("disputed_lines_named", |_s: &PayoutStatement, ctx| {
            let named = !input_of(ctx).disputed_lines.is_empty();
            Box::pin(async move {
                if named {
                    Ok(())
                } else {
                    Err(english("fnb.payout.problem.dispute_lines", &[]))
                }
            })
        })
        .effect(|s: &PayoutStatement, ctx, tx| {
            let id = s.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            let lines = input_of(ctx).disputed_lines;
            Box::pin(async move {
                for line in lines {
                    let flag = true;
                    PayoutLine::query()
                        .where_(PayoutLine::ID.eq(line))
                        .where_(PayoutLine::STATEMENT_ID.eq(id))
                        .update(&mut *tx, &[("disputed", &flag), ("dispute_note", &reason)])
                        .await?;
                }
                let at = Some(renox::db::now());
                set(tx, id, &[("dispute_reason", &reason), ("disputed_at", &at)]).await
            })
        })
}

fn resolve() -> Transition<PayoutStatement> {
    Transition::new(RESOLVE)
        .label("Resolve")
        .tone("primary")
        .permission(permissions::RECONCILIATION_MANAGE)
        .from([PayoutStatus::Disputed])
        .to(PayoutStatus::Matched)
        .reason_required()
        .effect(|s: &PayoutStatement, ctx, tx| {
            let id = s.id;
            let receivable = s.receivable_amount + input_of(ctx).adjustment;
            let note = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move {
                let at = Some(renox::db::now());
                set(
                    tx,
                    id,
                    &[
                        ("resolve_note", &note),
                        ("resolved_at", &at),
                        ("receivable_amount", &receivable),
                    ],
                )
                .await
            })
        })
}

fn settle() -> Transition<PayoutStatement> {
    Transition::new(SETTLE)
        .label("Settle")
        .system()
        .from([PayoutStatus::Matched])
        .to(PayoutStatus::Settled)
        .effect(|s: &PayoutStatement, _ctx, tx| {
            let id = s.id;
            Box::pin(async move {
                let at = Some(renox::db::now());
                set(tx, id, &[("settled_at", &at)]).await
            })
        })
}

/// What `void` and `void_approved` write besides the Void record.
fn void_rules(transition: Transition<PayoutStatement>) -> Transition<PayoutStatement> {
    transition.effect(|s: &PayoutStatement, ctx, tx| {
        let id = s.id;
        let reason = ctx.reason().map(|r| r.trim().to_owned());
        let by = ctx
            .actor()
            .map(|user| user.id)
            .or_else(|| ctx.meta().get("requester_id").and_then(Value::as_i64));
        Box::pin(async move {
            let at = Some(renox::db::now());
            set(
                tx,
                id,
                &[("void_reason", &reason), ("voided_at", &at), ("voided_by", &by)],
            )
            .await
        })
    })
}

impl StateMachine for PayoutStatement {
    type State = PayoutStatus;
    const COLUMN: renox::db::Col<PayoutStatement, PayoutStatus> = PayoutStatement::STATUS;

    fn machine() -> Machine<Self> {
        use PayoutStatus::{Disputed, Imported, Matched, Settled, Voided};
        let machine = Machine::new()
            .initial([Imported])
            .state(Imported)
            .tone("neutral")
            .state(Matched)
            .tone("info")
            .state(Disputed)
            .tone("warning")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(matching(MATCH, true))
            .transition(matching(MATCH_SYSTEM, false))
            .transition(dispute())
            .transition(resolve())
            .transition(settle());
        voids::transitions::<PayoutStatement, _>(SUBJECT_TYPE, [Imported, Matched, Disputed])
            .into_iter()
            .map(void_rules)
            .fold(machine, Machine::transition)
    }
}

/// The receivable booked on `match`: the posted effect of a statement.
async fn receivable_pending(tx: &mut Transaction, id: i64) -> Result<Vec<Effect>> {
    let Some(statement) = PayoutStatement::find(&mut *tx, id).await? else {
        return Ok(Vec::new());
    };
    if statement.receivable_booked_at.is_none() {
        return Ok(Vec::new());
    }
    Ok(vec![
        Effect::new(
            EffectKind::Posting,
            format!("receivable of {}", statement.number),
        )
        .amount(statement.receivable_amount),
    ])
}

/// Reverses the matching: the receivable goes, and so do the marks on the
/// orders whose compensation or cancelled line the statement matched.
async fn reverse_matching(
    tx: &mut Transaction,
    id: i64,
) -> Result<Vec<Effect>, voids::ReverseError> {
    let undone = receivable_pending(&mut *tx, id).await?;
    let no_time: Option<DateTime> = None;
    let no_statement: Option<i64> = None;
    let nothing: i64 = 0;
    set(
        &mut *tx,
        id,
        &[
            ("receivable_amount", &nothing),
            ("receivable_booked_at", &no_time),
            ("matched_at", &no_time),
        ],
    )
    .await?;
    ExpectedRefund::query()
        .where_(ExpectedRefund::STATEMENT_ID.eq(id))
        .update(
            &mut *tx,
            &[("matched_at", &no_time), ("statement_id", &no_statement)],
        )
        .await?;
    FnbOrder::query()
        .where_(FnbOrder::COMPENSATION_STATEMENT_ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("compensation_matched_at", &no_time),
                ("compensation_statement_id", &no_statement),
            ],
        )
        .await?;
    Ok(undone)
}

fn pending_effects<'a>(tx: &'a mut Transaction, id: i64) -> BoxFuture<'a, Result<Vec<Effect>>> {
    Box::pin(async move { receivable_pending(tx, id).await })
}

fn reverse_effects<'a>(
    tx: &'a mut Transaction,
    id: i64,
) -> BoxFuture<'a, Result<Vec<Effect>, voids::ReverseError>> {
    Box::pin(async move { reverse_matching(tx, id).await })
}

/// Registers the statement as the subject `fnb.payout_statement` with its own
/// approval action for the void, as voidable (permission
/// `reconciliation.manage`, the number kept) and for `rnx state:diagram`.
pub(crate) fn register() {
    // Declared before the `Voidable`, so the void policy takes the key.
    SubjectMachine::of::<PayoutStatement>(SUBJECT_TYPE)
        .action(voids::VOID, VOID_ACTION)
        .register();
    renox::state_machine::register::<PayoutStatement>();
    Voidable::of::<PayoutStatement>(SUBJECT_TYPE)
        .permission(permissions::RECONCILIATION_MANAGE)
        .number("fnb_payout_statement", |s: &PayoutStatement| {
            Some(s.number.clone())
        })
        .reverse(
            EffectKind::Posting,
            Reverse::new(pending_effects, reverse_effects),
        )
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_declares_the_five_states_and_the_transitions() {
        let machine = PayoutStatement::machine();
        assert_eq!(machine.states().len(), 5);
        for name in [
            MATCH,
            MATCH_SYSTEM,
            DISPUTE,
            RESOLVE,
            SETTLE,
            voids::VOID,
            voids::VOID_APPROVED,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        assert!(machine.find(SETTLE).unwrap().is_system());
        assert!(machine.find(MATCH_SYSTEM).unwrap().is_system());
        assert!(!machine.find(MATCH).unwrap().is_system());
        assert!(!machine.find(voids::VOID).unwrap().is_system());
    }

    #[test]
    fn a_settled_statement_cannot_be_voided() {
        let machine = PayoutStatement::machine();
        let void = machine.find(voids::VOID).unwrap();
        assert!(void.from_states().contains(&"matched".to_owned()));
        assert!(!void.from_states().contains(&"settled".to_owned()));
    }
}
