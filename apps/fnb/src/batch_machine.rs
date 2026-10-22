//! The production batch state machine (06 F&B §7.1 *Production batch*).
//!
//! ```text
//! [*] planned                                       (plan: the chef plans it; cancel deletes it)
//! planned --start (production.manage)--> in_production
//! in_production --finish (production.manage)--> produced
//! produced --hold (system: LotHeld)--> on_hold
//! on_hold --release (system: LotReleased)--> produced
//! produced --consume (system: stock reaches zero)--> consumed
//! produced --discard (production.manage; approval above AED 200)--> discarded
//! produced | on_hold --discard_system (system: LotDiscarded)--> discarded
//! ```
//!
//! `discard_approved` is the transition the kernel runs after the second
//! person approved a wastage above the limit (`STK.ACL.02`); the batch stays
//! `produced` while the request waits. A batch on hold is discarded only
//! through the *discard* decision of its stock hold line, so the hold's
//! approvals apply (`STK.ACL.05` for a recall): the lot's discard then emits
//! `LotDiscarded` and the system's `discard_system` follows.
//!
//! Renox guards get no `AppState`, so the services work out what a guard
//! needs (the lots, their status, the yield and the expiry) and pass it in a
//! [`BatchRun`]; the posting that reaches the stock module runs in
//! [`crate::batches`] around the transition, never inside an effect.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::machines::SubjectMachine;

use crate::batch::{BatchInput, BatchStatus, ProductionBatch, SUBJECT_TYPE};
use crate::permissions;
use crate::texts::english;

/// `start`: planned → in_production.
pub const START: &str = "start";
/// `finish`: in_production → produced.
pub const FINISH: &str = "finish";
/// `hold`: produced → on_hold, by the system.
pub const HOLD: &str = "hold";
/// `release`: on_hold → produced, by the system.
pub const RELEASE: &str = "release";
/// `consume`: produced → consumed, by the system.
pub const CONSUME: &str = "consume";
/// `discard`: produced → discarded.
pub const DISCARD: &str = "discard";
/// `discard_approved`: what the kernel runs after the second person said yes.
pub const DISCARD_APPROVED: &str = "discard_approved";
/// `discard_system`: produced, on_hold → discarded, by the system.
pub const DISCARD_SYSTEM: &str = "discard_system";
/// The approval action of a discard (`STK.ACL.02`).
pub const DISCARD_ACTION: &str = "fnb.batch_discard";

/// One ingredient lot the chef picked for a batch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickedLot {
    /// The ingredient (`inventory.item`).
    pub item_id: i64,
    /// The ingredient lot (`inventory.lot`).
    pub lot_id: i64,
    /// What goes into the batch, in thousandths.
    pub quantity_milli: i64,
}

/// What a run carries to the guards, effects and the approval policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchRun {
    /// The lots picked at `start`.
    #[serde(default)]
    pub picked: Vec<PickedLot>,
    /// What is wrong with the pick (a lot on hold, expired or short), in
    /// words; none when the lots are fine (`start`).
    #[serde(default)]
    pub problem: Option<String>,
    /// The yield that came out, in thousandths (`finish`).
    #[serde(default)]
    pub actual_yield_milli: i64,
    /// The expiry the chef entered (`finish`).
    #[serde(default)]
    pub expiry: Option<NaiveDate>,
    /// The earliest expiry of the ingredient lots used (`finish`).
    #[serde(default)]
    pub earliest_input_expiry: Option<NaiveDate>,
    /// The lot made by the stock module (`finish`).
    #[serde(default)]
    pub lot_id: Option<i64>,
    /// The reason the lot is on hold (`hold`).
    #[serde(default)]
    pub hold_reason: Option<String>,
    /// The value that would be wasted, in fils: what the approval policy
    /// reads (`discard`, `STK.ACL.02`).
    #[serde(default)]
    pub amount: i64,
    /// The user who runs it (who asked, when it waited for approval).
    #[serde(default)]
    pub by: Option<i64>,
}

fn run_of(ctx: &TransitionCtx) -> BatchRun {
    ctx.input_as::<BatchRun>().unwrap_or_default()
}

fn start() -> Transition<ProductionBatch> {
    Transition::new(START)
        .label("Start")
        .tone("primary")
        .permission(permissions::PRODUCTION_MANAGE)
        .from([BatchStatus::Planned])
        .to(BatchStatus::InProduction)
        .guard("lots_are_usable", |_batch: &ProductionBatch, ctx| {
            let run = run_of(ctx);
            let problem = match (run.problem, run.picked.is_empty()) {
                (Some(problem), _) => Some(problem),
                (None, true) => Some(english("fnb.batch.problem.no_lots", &[])),
                (None, false) => None,
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|batch: &ProductionBatch, ctx, tx| {
            let id = batch.id;
            let run = run_of(ctx);
            Box::pin(async move {
                for pick in &run.picked {
                    let mut row = BatchInput {
                        batch_id: id,
                        item_id: pick.item_id,
                        lot_id: pick.lot_id,
                        quantity_milli: pick.quantity_milli,
                        ..BatchInput::default()
                    };
                    row.insert(&mut *tx).await?;
                }
                let at = Some(renox::db::now());
                ProductionBatch::query()
                    .where_(ProductionBatch::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("started_at", &at), ("started_by", &run.by)],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn finish() -> Transition<ProductionBatch> {
    Transition::new(FINISH)
        .label("Finish")
        .tone("success")
        .permission(permissions::PRODUCTION_MANAGE)
        .from([BatchStatus::InProduction])
        .to(BatchStatus::Produced)
        .guard("yield_recorded", |_batch: &ProductionBatch, ctx| {
            let run = run_of(ctx);
            Box::pin(async move {
                if run.actual_yield_milli > 0 {
                    Ok(())
                } else {
                    Err(english("fnb.batch.problem.yield", &[]))
                }
            })
        })
        .guard("expiry_not_later_than_inputs", |_batch: &ProductionBatch, ctx| {
            let run = run_of(ctx);
            Box::pin(async move {
                let Some(expiry) = run.expiry else {
                    return Err(english("fnb.batch.problem.expiry_needed", &[]));
                };
                match run.earliest_input_expiry {
                    Some(earliest) if expiry > earliest => Err(english(
                        "fnb.batch.problem.expiry_later",
                        &[("expiry", &expiry), ("earliest", &earliest)],
                    )),
                    _ => Ok(()),
                }
            })
        })
        .effect(|batch: &ProductionBatch, ctx, tx| {
            let id = batch.id;
            let run = run_of(ctx);
            Box::pin(async move {
                let at = Some(renox::db::now());
                let yielded = Some(run.actual_yield_milli);
                ProductionBatch::query()
                    .where_(ProductionBatch::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("actual_yield_milli", &yielded),
                            ("expiry_date", &run.expiry),
                            ("lot_id", &run.lot_id),
                            ("finished_at", &at),
                            ("finished_by", &run.by),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

fn hold() -> Transition<ProductionBatch> {
    Transition::new(HOLD)
        .label("Hold")
        .tone("warning")
        .system()
        .from([BatchStatus::Produced])
        .to(BatchStatus::OnHold)
        .effect(|batch: &ProductionBatch, ctx, tx| {
            let id = batch.id;
            let reason = run_of(ctx).hold_reason;
            Box::pin(async move {
                ProductionBatch::query()
                    .where_(ProductionBatch::ID.eq(id))
                    .update(&mut *tx, &[("hold_reason", &reason)])
                    .await?;
                Ok(())
            })
        })
}

fn release() -> Transition<ProductionBatch> {
    Transition::new(RELEASE)
        .label("Release")
        .tone("success")
        .system()
        .from([BatchStatus::OnHold])
        .to(BatchStatus::Produced)
        .effect(|batch: &ProductionBatch, _ctx, tx| {
            let id = batch.id;
            Box::pin(async move {
                let none: Option<String> = None;
                ProductionBatch::query()
                    .where_(ProductionBatch::ID.eq(id))
                    .update(&mut *tx, &[("hold_reason", &none)])
                    .await?;
                Ok(())
            })
        })
}

fn consume() -> Transition<ProductionBatch> {
    Transition::new(CONSUME)
        .label("Consume")
        .system()
        .from([BatchStatus::Produced])
        .to(BatchStatus::Consumed)
}

/// The effect of every discard: the reason is kept on the batch.
fn discard_effect(
    transition: Transition<ProductionBatch>,
) -> Transition<ProductionBatch> {
    transition.effect(|batch: &ProductionBatch, ctx, tx| {
        let id = batch.id;
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            ProductionBatch::query()
                .where_(ProductionBatch::ID.eq(id))
                .update(&mut *tx, &[("discard_reason", &reason)])
                .await?;
            Ok(())
        })
    })
}

fn discard() -> Transition<ProductionBatch> {
    discard_effect(
        Transition::new(DISCARD)
            .label("Discard")
            .tone("danger")
            .permission(permissions::PRODUCTION_MANAGE)
            .reason_required()
            .from([BatchStatus::Produced])
            .to(BatchStatus::Discarded),
    )
}

fn discard_approved() -> Transition<ProductionBatch> {
    discard_effect(
        Transition::new(DISCARD_APPROVED)
            .label("Discard (approved)")
            .tone("danger")
            .system()
            .from([BatchStatus::Produced])
            .to(BatchStatus::Discarded),
    )
}

fn discard_system() -> Transition<ProductionBatch> {
    discard_effect(
        Transition::new(DISCARD_SYSTEM)
            .label("Discard (lot discarded)")
            .tone("danger")
            .system()
            .from([BatchStatus::Produced, BatchStatus::OnHold])
            .to(BatchStatus::Discarded),
    )
}

impl StateMachine for ProductionBatch {
    type State = BatchStatus;
    const COLUMN: renox::db::Col<ProductionBatch, BatchStatus> = ProductionBatch::STATUS;

    fn machine() -> Machine<Self> {
        use BatchStatus::{Consumed, Discarded, InProduction, OnHold, Planned, Produced};
        Machine::new()
            .initial([Planned])
            .state(Planned)
            .tone("neutral")
            .state(InProduction)
            .tone("info")
            .state(Produced)
            .tone("success")
            .state(OnHold)
            .tone("warning")
            .state(Consumed)
            .tone("success")
            .terminal()
            .state(Discarded)
            .tone("error")
            .terminal()
            .transition(start())
            .transition(finish())
            .transition(hold())
            .transition(release())
            .transition(consume())
            .transition(discard())
            .transition(discard_approved())
            .transition(discard_system())
    }
}

/// Registers the batch as the subject `fnb.batch` with the approval action of
/// its discard (`STK.ACL.02`: a wastage above `limit` fils needs a second
/// holder of `stock.approve`, never the chef who recorded it).
pub(crate) fn register(limit: i64) {
    SubjectMachine::of::<ProductionBatch>(SUBJECT_TYPE)
        .action(DISCARD, DISCARD_ACTION)
        .scope(|_batch: &ProductionBatch| renox::auth::permissions::Scope::global())
        .register();
    wl_kernel::PolicyDefault::new(DISCARD_ACTION)
        .amount_above(limit)
        .step_permission(permissions::STOCK_APPROVE)
        .register();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_declares_the_six_states_and_the_transitions() {
        let machine = ProductionBatch::machine();
        assert_eq!(machine.states().len(), 6);
        for name in [
            START,
            FINISH,
            HOLD,
            RELEASE,
            CONSUME,
            DISCARD,
            DISCARD_APPROVED,
            DISCARD_SYSTEM,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn only_the_chef_starts_finishes_and_discards_and_the_system_does_the_rest() {
        let machine = ProductionBatch::machine();
        for name in [START, FINISH, DISCARD] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
        for name in [HOLD, RELEASE, CONSUME, DISCARD_APPROVED, DISCARD_SYSTEM] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
    }

    #[test]
    fn a_batch_on_hold_is_discarded_only_by_the_system() {
        let machine = ProductionBatch::machine();
        assert_eq!(machine.find(DISCARD).unwrap().from_states(), ["produced"]);
        assert_eq!(
            machine.find(DISCARD_SYSTEM).unwrap().from_states(),
            ["produced", "on_hold"]
        );
    }
}
