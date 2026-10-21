//! Deciding the lines of a stock hold (wl-inventory §4 *Stock hold*, issue
//! #231). A hold line is a record with a machine of its own, so that a
//! decision can wait for its approval and still count as undecided:
//!
//! ```text
//! undecided --release / release_approved-----------> released
//!           |--discard / discard_approved-----------> discarded
//!           |--return_to_supplier-------------------> returned_to_supplier
//! ```
//!
//! - `release` (a holder of `lots.release` in the location's scope): the
//!   quantity is available again. For a listed hold reason (`recall` by
//!   default, `STK.ACL.05`) the run becomes an approval request for a second
//!   holder of `lots.release`, never the decider; the system's
//!   `release_approved` follows the last yes. A line of an `expired` lot is
//!   not offered `release`.
//! - `discard`: a `wastage` movement takes the quantity out with the hold's
//!   reason, charged (by the listener, after the commit) to the recall when
//!   the hold is a recall's, else to the hold. Above the wastage value
//!   (`STK.ACL.02`, [`crate::InventoryConfig::wastage_approval_fils`]) it
//!   waits for a second holder, and nothing posts until `discard_approved`.
//! - `return_to_supplier`: a movement takes the quantity to the quarantine
//!   location, where it is reserved by a draft Return to supplier for the
//!   lot's supplier: the draft the supplier has already, else a new one.
//!   Not offered for a lot with no supplier (a produced lot).
//!
//! The lot, the closing of the hold and the notices follow the transition
//! in [`crate::hold_decide::settle`], called by the listener.
//!
//! Decisions: a `discard` takes the stock out as a movement from the line's
//! location without a destination (the scrap, as with a `return_out`), so
//! the lot's remaining quantity falls; the quarantine move has both ends,
//! the lot keeps its quantity; a line whose quantity is no longer at its
//! location (counted away meanwhile) is refused until it is counted.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;

use crate::balance;
use crate::item::{Item, value_of};
use crate::location::quarantine_location;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::receipt_post::SUPPLIER_RETURN_SERIES;
use crate::stock_hold::{HoldLineStatus, StockHold, StockHoldLine};
use crate::supplier_return::{
    RejectedHandling, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine,
    WantedOutcome, lines_in,
};
use crate::texts::{english, said};

/// The user's `release` transition's name.
pub const RELEASE: &str = "release";

/// The system's `release_approved` transition's name: the kernel runs it
/// when the second person approves a release.
pub const RELEASE_APPROVED: &str = "release_approved";

/// The user's `discard` transition's name.
pub const DISCARD: &str = "discard";

/// The system's `discard_approved` transition's name.
pub const DISCARD_APPROVED: &str = "discard_approved";

/// The user's `return_to_supplier` transition's name.
pub const RETURN_TO_SUPPLIER: &str = "return_to_supplier";

/// The approval policy's action key of `release` (`STK.ACL.05`).
pub const RELEASE_ACTION: &str = "inventory.hold_release";

/// The approval policy's action key of `discard` (`STK.ACL.02`).
pub const DISCARD_ACTION: &str = "inventory.hold_discard";

/// The key of the release input the `STK.ACL.05` policy reads: above 0 when
/// the hold's reason needs a second person.
pub const SECOND_PERSON_KEY: &str = "second_person";

/// What a decision carries to its guards, its effect and the approval
/// policies. The kernel keeps it with an approval request and hands it to
/// the `_approved` run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DecideInput {
    /// The value of the line at the average cost, in fils: what the
    /// `STK.ACL.02` condition reads.
    pub amount: i64,
    /// 1 when the hold's reason is listed (`STK.ACL.05`), else 0.
    pub second_person: i64,
    /// The user who decides (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// Whether the hold is still open.
    pub hold_open: bool,
    /// Whether the lot is `expired`.
    pub lot_expired: bool,
    /// The supplier of the lot, when it has one.
    pub supplier: Option<i64>,
    /// When it is decided.
    pub at: Option<DateTime>,
}

fn input(ctx: &TransitionCtx) -> Option<DecideInput> {
    ctx.input_as::<DecideInput>().ok()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Records the decision on the line: the outcome, who and when, and the
/// draft return a line returned to the supplier is on.
async fn decided_in(
    tx: &mut Transaction,
    line_id: i64,
    outcome: &str,
    run: &DecideInput,
    supplier_return_id: Option<i64>,
) -> Result {
    let now = renox::db::now();
    StockHoldLine::query()
        .where_(StockHoldLine::ID.eq(line_id))
        .update(
            &mut *tx,
            &[
                ("outcome", &Some(outcome.to_owned())),
                ("decided_by", &run.by),
                ("decided_at", &Some(run.at.unwrap_or(now))),
                ("supplier_return_id", &supplier_return_id),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The effect of `release`: nothing moves.
async fn release_in(tx: &mut Transaction, line: StockHoldLine, run: &DecideInput) -> Result {
    decided_in(tx, line.id, "release", run, None).await
}

/// The effect of `discard`: the wastage movement.
async fn discard_in(tx: &mut Transaction, line: StockHoldLine, run: &DecideInput) -> Result {
    let item = Item::find_or_404(&mut *tx, line.item_id).await?;
    let hold = StockHold::find_or_404(&mut *tx, line.stock_hold_id).await?;
    let on_hand = balance::on_hand_at(&mut *tx, item.id, line.location_id, line.lot_id).await?;
    if on_hand < line.quantity_milli {
        return Err(refused(said("wl_inventory.hold.stock_moved", &[])));
    }
    let mut moved = StockMovement::leaving(
        MovementType::Wastage,
        item.id,
        line.lot_id,
        line.location_id,
        line.quantity_milli,
        item.average_cost,
        value_of(line.quantity_milli, item.average_cost),
        &line.subject(),
        Some(hold.reason.clone()),
        run.by,
    );
    moved.moved_at = run.at.unwrap_or_else(renox::db::now);
    moved.insert(&mut *tx).await?;
    decided_in(tx, line.id, "discard", run, None).await
}

/// Adds `line` to the draft return of `supplier` that waits in quarantine,
/// or makes one; returns its id.
async fn add_to_draft_return(
    tx: &mut Transaction,
    supplier: i64,
    hold: &StockHold,
    quarantine: i64,
    line: &StockHoldLine,
    item: &Item,
) -> Result<i64> {
    let draft = SupplierReturn::query()
        .where_(SupplierReturn::STATUS.eq(ReturnStatus::Draft))
        .where_(SupplierReturn::SUPPLIER_PARTY_ID.eq(supplier))
        .where_(SupplierReturn::LOCATION_ID.eq(Some(quarantine)))
        .order_by(SupplierReturn::ID)
        .first(&mut *tx)
        .await?;
    let ret = match draft {
        Some(found) => found,
        None => {
            let source = hold.subject();
            let mut ret = SupplierReturn {
                supplier_party_id: supplier,
                source_type: source.subject_type.clone(),
                source_id: source.subject_id,
                handling: RejectedHandling::KeptForCollection,
                location_id: Some(quarantine),
                status: ReturnStatus::Draft,
                ..SupplierReturn::default()
            };
            ret.number = Some(
                SUPPLIER_RETURN_SERIES
                    .next(&mut *tx, None, None, today())
                    .await?,
            );
            ret.insert(&mut *tx).await?;
            ret
        }
    };
    let position = lines_in(&mut *tx, ret.id).await?.len() as i64 + 1;
    let mut row = SupplierReturnLine {
        supplier_return_id: ret.id,
        position,
        item_id: item.id,
        lot_id: line.lot_id,
        quantity_milli: line.quantity_milli,
        unit_cost: item.average_cost,
        reason: hold.reason.clone(),
        wanted_outcome: WantedOutcome::Credit,
        origin: ReturnOrigin::FromStock,
        ..SupplierReturnLine::default()
    };
    row.insert(&mut *tx).await?;
    Ok(ret.id)
}

/// The effect of `return_to_supplier`: the quantity goes to quarantine and
/// onto a draft return.
async fn return_in(tx: &mut Transaction, line: StockHoldLine, run: &DecideInput) -> Result {
    let Some(supplier) = run.supplier else {
        return Err(refused(said("wl_inventory.hold.no_supplier", &[])));
    };
    let item = Item::find_or_404(&mut *tx, line.item_id).await?;
    let hold = StockHold::find_or_404(&mut *tx, line.stock_hold_id).await?;
    let on_hand = balance::on_hand_at(&mut *tx, item.id, line.location_id, line.lot_id).await?;
    if on_hand < line.quantity_milli {
        return Err(refused(said("wl_inventory.hold.stock_moved", &[])));
    }
    let quarantine = quarantine_location(tx).await?;
    let mut moved = StockMovement::moving(
        MovementType::TransferOut,
        item.id,
        line.lot_id,
        Some(line.location_id),
        Some(quarantine),
        line.quantity_milli,
        item.average_cost,
        value_of(line.quantity_milli, item.average_cost),
        &line.subject(),
        run.by,
    );
    moved.reason = Some(hold.reason.clone());
    moved.moved_at = run.at.unwrap_or_else(renox::db::now);
    moved.insert(&mut *tx).await?;
    let return_id = add_to_draft_return(tx, supplier, &hold, quarantine, &line, &item).await?;
    decided_in(tx, line.id, "return_to_supplier", run, Some(return_id)).await
}

/// The rules every decision shares: from `undecided`, the hold open.
fn decision_rules(
    transition: Transition<StockHoldLine>,
    to: HoldLineStatus,
) -> Transition<StockHoldLine> {
    transition.from([HoldLineStatus::Undecided]).to(to).guard(
        "hold_open",
        |_line: &StockHoldLine, ctx| {
            let open = input(ctx).is_some_and(|run| run.hold_open);
            Box::pin(async move {
                if open {
                    Ok(())
                } else {
                    Err(english("wl_inventory.hold.not_open").to_owned())
                }
            })
        },
    )
}

/// `release` and `release_approved`.
fn release_rules(transition: Transition<StockHoldLine>) -> Transition<StockHoldLine> {
    decision_rules(transition, HoldLineStatus::Released)
        .guard("lot_not_expired", |_line: &StockHoldLine, ctx| {
            let expired = input(ctx).is_none_or(|run| run.lot_expired);
            Box::pin(async move {
                if expired {
                    Err(english("wl_inventory.hold.expired_lot").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|line: &StockHoldLine, ctx, tx| {
            let line = line.clone();
            let run = input(ctx);
            Box::pin(async move {
                let run = run.ok_or_else(|| refused(said("wl_inventory.hold.not_open", &[])))?;
                release_in(tx, line, &run).await
            })
        })
}

/// `discard` and `discard_approved`.
fn discard_rules(transition: Transition<StockHoldLine>) -> Transition<StockHoldLine> {
    decision_rules(transition, HoldLineStatus::Discarded).effect(|line: &StockHoldLine, ctx, tx| {
        let line = line.clone();
        let run = input(ctx);
        Box::pin(async move {
            let run = run.ok_or_else(|| refused(said("wl_inventory.hold.not_open", &[])))?;
            discard_in(tx, line, &run).await
        })
    })
}

impl StateMachine for StockHoldLine {
    type State = HoldLineStatus;
    const COLUMN: renox::db::Col<StockHoldLine, HoldLineStatus> = StockHoldLine::STATUS;

    fn machine() -> Machine<Self> {
        use HoldLineStatus::{Discarded, Released, ReturnedToSupplier, Undecided};
        Machine::new()
            .initial([Undecided])
            .state(Undecided)
            .tone("warning")
            .state(Released)
            .tone("success")
            .terminal()
            .state(Discarded)
            .tone("neutral")
            .terminal()
            .state(ReturnedToSupplier)
            .tone("info")
            .terminal()
            .transition(release_rules(
                Transition::new(RELEASE)
                    .label("Release")
                    .tone("primary")
                    .permission_in(permissions::LOTS_RELEASE, |line: &StockHoldLine| {
                        line.scope()
                    }),
            ))
            .transition(release_rules(
                Transition::new(RELEASE_APPROVED)
                    .label("Release (approved)")
                    .system(),
            ))
            .transition(discard_rules(
                Transition::new(DISCARD)
                    .label("Discard")
                    .tone("danger")
                    .permission_in(permissions::LOTS_RELEASE, |line: &StockHoldLine| {
                        line.scope()
                    }),
            ))
            .transition(discard_rules(
                Transition::new(DISCARD_APPROVED)
                    .label("Discard (approved)")
                    .system(),
            ))
            .transition(
                decision_rules(
                    Transition::new(RETURN_TO_SUPPLIER)
                        .label("Return to supplier")
                        .permission_in(permissions::LOTS_RELEASE, |line: &StockHoldLine| {
                            line.scope()
                        }),
                    ReturnedToSupplier,
                )
                .guard("lot_has_supplier", |_line: &StockHoldLine, ctx| {
                    let none = input(ctx).is_none_or(|run| run.supplier.is_none());
                    Box::pin(async move {
                        if none {
                            Err(english("wl_inventory.hold.no_supplier").to_owned())
                        } else {
                            Ok(())
                        }
                    })
                })
                .effect(|line: &StockHoldLine, ctx, tx| {
                    let line = line.clone();
                    let run = input(ctx);
                    Box::pin(async move {
                        let run =
                            run.ok_or_else(|| refused(said("wl_inventory.hold.not_open", &[])))?;
                        return_in(tx, line, &run).await
                    })
                }),
            )
    }
}

/// The subject of `line`, for the callers that only have its id.
pub fn subject_of(id: i64) -> Subject {
    Subject::new(wl_kernel::inventory::STOCK_HOLD_LINE_SUBJECT_TYPE, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_decided_once_and_a_second_person_can_finish_a_release_or_a_discard() {
        let machine = StockHoldLine::machine();
        assert_eq!(machine.initial_states(), ["undecided"]);
        for name in [RELEASE, DISCARD, RETURN_TO_SUPPLIER] {
            let found = machine.find(name).expect(name);
            assert!(!found.is_system(), "{name}");
            assert_eq!(found.from_states(), ["undecided"], "{name}");
        }
        for name in [RELEASE_APPROVED, DISCARD_APPROVED] {
            assert!(machine.find(name).expect(name).is_system(), "{name}");
        }
        assert_eq!(
            machine.find(RELEASE).expect("release").guard_names(),
            ["hold_open", "lot_not_expired"]
        );
        assert_eq!(
            machine
                .find(RETURN_TO_SUPPLIER)
                .expect("return")
                .guard_names(),
            ["hold_open", "lot_has_supplier"]
        );
        assert_eq!(
            machine.find(DISCARD).expect("discard").to_state(),
            Some("discarded")
        );
        for state in ["released", "discarded", "returned_to_supplier"] {
            let ended = machine.states().iter().find(|s| s.name == state);
            assert!(ended.is_some_and(|s| s.terminal), "{state}");
        }
    }

    #[test]
    fn the_subject_of_a_line_is_an_inventory_one() {
        assert_eq!(subject_of(7).to_string(), "inventory.stock_hold_line:7");
    }
}
