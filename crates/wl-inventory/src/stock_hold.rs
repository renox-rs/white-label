//! The Stock hold (wl-inventory §3, §4 *Stock hold*, issue #230): stock is
//! blocked without moving it, whole lots (every location, the lot goes
//! `on_hold`) or the stock at one location (the lot status stays as it is;
//! only that balance is held). Held quantity is not available for picking,
//! issue, consumption, transfer or dispatch ([`crate::balance`]).
//!
//! ```text
//! [*] --place--> open --add_lines--> open
//!                  |--close (system, last line decided)--> closed
//!                  |--cancel (lots.release, nothing decided)--> cancelled
//! ```
//!
//! This file has the rows and the machine; `place`, the notices and the
//! system service are in [`crate::hold_service`], the decisions on the lines
//! in [`crate::hold_line`] and [`crate::hold_decide`].
//!
//! - `place` creates the hold open, with its lines (item, location, lot,
//!   quantity) made from the balances in scope.
//! - `add_lines` (a holder of `stock.hold`) and `add_lines_by_system` (a
//!   goods receipt's `post` or a transfer's `receive` into the location of
//!   an open hold on a location): the quantity moved in joins the hold as new
//!   lines with the hold's reason and reference.
//!
//! - `close` (system): runs when the last line is decided and its effect
//!   posted ([`crate::hold_decide::settle`]); `StockHoldClosed` follows.
//! - `cancel` (`lots.release` in the hold's scope, a reason): a hold placed
//!   in error, before any line is decided; never a recall hold. The held
//!   quantity is available again and a lot goes back to the status it had
//!   ([`crate::hold_decide::cancel`]).
//!
//! Decisions: the reason is one of [`REASONS`]; a reference is required (§4
//! guard "reason and reference"); a hold on a location with no items named
//! covers every item there, else only the named ones; the lines of a second
//! hold on a lot that is `on_hold` already are added and the lot stays as it
//! is; the balance of an `expired` lot is a line too and the lot stays
//! `expired`.

use renox::auth::permissions::Scope;
use renox::db::{Executor, Transaction};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{STOCK_HOLD_LINE_SUBJECT_TYPE, STOCK_HOLD_SUBJECT_TYPE};
use wl_kernel::numbering::{Reset, Series};

use crate::location::{Location, scope_of};
use crate::permissions;
use crate::texts::english;

/// The user's `add_lines` transition's name.
pub const ADD_LINES: &str = "add_lines";

/// The system's `add_lines_by_system` transition's name.
pub const ADD_LINES_BY_SYSTEM: &str = "add_lines_by_system";

/// The system's `close` transition's name.
pub const CLOSE: &str = "close";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The reasons a hold can have (§7 *Stock hold reasons*).
pub const REASONS: [&str; 5] = ["recall", "quality", "temperature", "complaint", "other"];

/// The notification type told to the deciders when a hold is placed.
pub const PLACED_NOTICE: &str = "inventory.stock_hold_placed";

/// The notification type told to the deciders when lines are added.
pub const LINES_NOTICE: &str = "inventory.stock_hold_lines";

/// The numbers of stock holds: `HLD-2026-0001`.
pub const STOCK_HOLD_SERIES: Series = Series {
    key: "stock_hold",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "HLD",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a stock hold stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoldStatus {
    /// Stock blocked; lines waiting for a decision.
    #[default]
    Open,
    /// Every line decided and its effect posted (terminal).
    Closed,
    /// Placed in error; stock available again (terminal).
    Cancelled,
}

/// What a hold covers.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoldScope {
    /// Whole lots, wherever they are; each lot goes `on_hold`.
    #[default]
    Lots,
    /// The stock at one location, all items or the ones named.
    Location,
}

/// A block on stock pending a decision.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_stock_holds",
    state_machine,
    unique(number),
    index(status)
)]
pub struct StockHold {
    /// The primary key.
    pub id: i64,
    /// The number from the `HLD-` series.
    pub number: Option<String>,
    /// `recall`, `quality`, `temperature`, `complaint` or `other`.
    pub reason: String,
    /// The type of the record that caused it.
    pub reference_type: Option<String>,
    /// The id of the record that caused it.
    pub reference_id: Option<i64>,
    /// Whole lots, or a location.
    pub scope_kind: HoldScope,
    /// The location, for a hold on a location.
    pub location_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: HoldStatus,
    /// The user who placed it; `None` for the system.
    pub placed_by: Option<i64>,
    /// The type of the scope of the location, for a hold on a location.
    pub scope_type: Option<String>,
    /// The id of the scope of the location, for a hold on a location.
    pub scope_id: Option<i64>,
    /// When it was placed.
    pub placed_at: Option<DateTime>,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who cancelled it.
    pub cancelled_by: Option<i64>,
    /// When it closed or was cancelled.
    pub closed_at: Option<DateTime>,
    /// The reminders to the deciders sent so far (`STK.RUL.07`).
    pub reminders_sent: i64,
    /// When the last reminder went out.
    pub last_reminded_at: Option<DateTime>,
    /// The notices to the escalation role sent so far.
    pub escalations_sent: i64,
    /// When the last escalation went out.
    pub last_escalated_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockHold {
    /// Who may cancel it: the scope of the location for a hold on a
    /// location, everywhere for a hold on whole lots.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }

    /// The hold as a [`Subject`]: `inventory.stock_hold:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(STOCK_HOLD_SUBJECT_TYPE, self.id)
    }

    /// The record that caused it.
    pub fn reference(&self) -> Option<Subject> {
        match (&self.reference_type, self.reference_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

/// Where a held line stands (a decision waiting for its approval leaves it
/// `undecided`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HoldLineStatus {
    /// Waiting for a decision; the quantity is held.
    #[default]
    Undecided,
    /// Decided `release`: available again (terminal).
    Released,
    /// Decided `discard`: posted as wastage (terminal).
    Discarded,
    /// Decided `return_to_supplier`: in quarantine, on a draft return
    /// (terminal).
    ReturnedToSupplier,
}

/// One held quantity: an item at a location, of a lot when it is tracked.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_stock_hold_lines",
    state_machine,
    index(stock_hold_id),
    index(status)
)]
pub struct StockHoldLine {
    /// The primary key.
    pub id: i64,
    /// The hold.
    pub stock_hold_id: i64,
    /// The item.
    pub item_id: i64,
    /// The location the quantity is at.
    pub location_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity held, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: HoldLineStatus,
    /// The type of the scope of the location (who may decide the line).
    pub scope_type: Option<String>,
    /// The id of the scope of the location.
    pub scope_id: Option<i64>,
    /// The draft return a `return_to_supplier` line was added to.
    pub supplier_return_id: Option<i64>,
    /// `release`, `discard` or `return_to_supplier` once decided.
    pub outcome: Option<String>,
    /// The user who decided it.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockHoldLine {
    /// The line as a [`Subject`]: `inventory.stock_hold_line:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(STOCK_HOLD_LINE_SUBJECT_TYPE, self.id)
    }

    /// Who may decide it: the scope of its location.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }
}

/// Inserts a line for `quantity_milli` of `item_id` (of `lot_id`) at
/// `location_id` into hold `hold_id`, with the scope of the location.
///
/// # Errors
///
/// A database error.
pub(crate) async fn new_line(
    tx: &mut Transaction,
    hold_id: i64,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
    quantity_milli: i64,
) -> Result<StockHoldLine> {
    let location = Location::find_or_404(&mut *tx, location_id).await?;
    let mut row = StockHoldLine {
        stock_hold_id: hold_id,
        item_id,
        location_id,
        lot_id,
        quantity_milli,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        ..StockHoldLine::default()
    };
    row.insert(&mut *tx).await?;
    Ok(row)
}

/// An item a hold on a location is limited to.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_stock_hold_items", unique(stock_hold_id, item_id))]
pub struct StockHoldItem {
    /// The primary key.
    pub id: i64,
    /// The hold.
    pub stock_hold_id: i64,
    /// The item.
    pub item_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A quantity that joins an open hold on a location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedLine {
    /// The item.
    pub item_id: i64,
    /// The location the quantity moved to.
    pub location_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

/// What an `add_lines` run carries to its guards and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AddLinesInput {
    /// The lines to add.
    pub lines: Vec<AddedLine>,
    /// The ids of the items the hold is limited to; empty for all items.
    pub scope_items: Vec<i64>,
}

fn input(ctx: &TransitionCtx) -> Option<AddLinesInput> {
    ctx.input_as::<AddLinesInput>().ok()
}

/// What a `close` or `cancel` run carries to its guards.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SettleInput {
    /// The lines still undecided.
    pub undecided: i64,
    /// The lines decided already.
    pub decided: i64,
}

fn settle_input(ctx: &TransitionCtx) -> Option<SettleInput> {
    ctx.input_as::<SettleInput>().ok()
}

/// The effect of `close` and `cancel`: when it ended, and for a cancel why
/// and by whom (columns other than the status).
async fn ended_in(
    tx: &mut Transaction,
    id: i64,
    reason: Option<String>,
    by: Option<i64>,
) -> Result {
    let now = renox::db::now();
    StockHold::query()
        .where_(StockHold::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("closed_at", &Some(now)),
                ("cancel_reason", &reason),
                ("cancelled_by", &by),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The two `add_lines` transitions: the user's and the system's.
fn add_lines_rules(transition: Transition<StockHold>) -> Transition<StockHold> {
    transition
        .from([HoldStatus::Open])
        .to(HoldStatus::Open)
        .guard("location_scope", |hold: &StockHold, ctx| {
            let location = hold.location_id;
            let scope = hold.scope_kind;
            let wrong = input(ctx).is_none_or(|input| {
                scope != HoldScope::Location
                    || input.lines.is_empty()
                    || input
                        .lines
                        .iter()
                        .any(|line| Some(line.location_id) != location)
            });
            Box::pin(async move {
                if wrong {
                    Err(english("wl_inventory.hold.not_location_scope").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .guard("item_in_scope", |_hold: &StockHold, ctx| {
            let wrong = input(ctx).is_none_or(|input| {
                !input.scope_items.is_empty()
                    && input
                        .lines
                        .iter()
                        .any(|line| !input.scope_items.contains(&line.item_id))
            });
            Box::pin(async move {
                if wrong {
                    Err(english("wl_inventory.hold.item_not_in_scope").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .guard("quantity_moved_in", |_hold: &StockHold, ctx| {
            let wrong = input(ctx).is_none_or(|input| {
                input.lines.iter().any(|line| line.quantity_milli <= 0)
            });
            Box::pin(async move {
                if wrong {
                    Err(english("wl_inventory.hold.no_quantity").to_owned())
                } else {
                    Ok(())
                }
            })
        })
        .effect(|hold: &StockHold, ctx, tx| {
            let hold_id = hold.id;
            let lines = input(ctx).map(|input| input.lines).unwrap_or_default();
            Box::pin(async move {
                for line in lines {
                    new_line(
                        &mut *tx,
                        hold_id,
                        line.item_id,
                        line.location_id,
                        line.lot_id,
                        line.quantity_milli,
                    )
                    .await?;
                }
                Ok(())
            })
        })
}

impl StateMachine for StockHold {
    type State = HoldStatus;
    const COLUMN: renox::db::Col<StockHold, HoldStatus> = StockHold::STATUS;

    fn machine() -> Machine<Self> {
        use HoldStatus::{Cancelled, Closed, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(add_lines_rules(
                Transition::new(ADD_LINES)
                    .label("Add lines")
                    .permission(permissions::STOCK_HOLD),
            ))
            .transition(add_lines_rules(
                Transition::new(ADD_LINES_BY_SYSTEM)
                    .label("Add lines (moved in)")
                    .system(),
            ))
            .transition(
                Transition::new(CLOSE)
                    .label("Close")
                    .tone("success")
                    .system()
                    .from([Open])
                    .to(Closed)
                    .guard("every_line_decided", |_hold: &StockHold, ctx| {
                        let open = settle_input(ctx).is_none_or(|input| input.undecided > 0);
                        Box::pin(async move {
                            if open {
                                Err(english("wl_inventory.hold.lines_undecided").to_owned())
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|hold: &StockHold, _ctx, tx| {
                        let id = hold.id;
                        Box::pin(async move { ended_in(tx, id, None, None).await })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("danger")
                    .permission_in(permissions::LOTS_RELEASE, |hold: &StockHold| hold.scope())
                    .reason_required()
                    .from([Open])
                    .to(Cancelled)
                    .guard("not_a_recall", |hold: &StockHold, _ctx| {
                        let recall = hold.reason == "recall";
                        Box::pin(async move {
                            if recall {
                                Err(english("wl_inventory.hold.recall_cannot_cancel").to_owned())
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .guard("no_line_decided", |_hold: &StockHold, ctx| {
                        let decided = settle_input(ctx).is_none_or(|input| input.decided > 0);
                        Box::pin(async move {
                            if decided {
                                Err(english("wl_inventory.hold.line_decided").to_owned())
                            } else {
                                Ok(())
                            }
                        })
                    })
                    .effect(|hold: &StockHold, ctx, tx| {
                        let id = hold.id;
                        let reason = ctx.reason().map(str::to_owned);
                        let by = ctx.actor().map(|user| user.id);
                        Box::pin(async move { ended_in(tx, id, reason, by).await })
                    }),
            )
    }
}
/// The lines of `hold`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn lines_of<'c>(db: impl Executor<'c>, hold_id: i64) -> Result<Vec<StockHoldLine>> {
    StockHoldLine::query()
        .where_(StockHoldLine::STOCK_HOLD_ID.eq(hold_id))
        .order_by(StockHoldLine::ID)
        .get(db)
        .await
}

/// The ids of the items hold `hold_id` is limited to; empty for all items.
pub(crate) async fn scope_items<'c>(db: impl Executor<'c>, hold_id: i64) -> Result<Vec<i64>> {
    Ok(StockHoldItem::query()
        .where_(StockHoldItem::STOCK_HOLD_ID.eq(hold_id))
        .order_by(StockHoldItem::ID)
        .get(db)
        .await?
        .iter()
        .map(|item| item.item_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_open_until_decided_and_adds_lines_to_itself() {
        let machine = StockHold::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let names: Vec<_> = machine.states().iter().map(|s| s.name.as_str()).collect();
        for state in ["open", "closed", "cancelled"] {
            assert!(names.contains(&state), "{state}");
        }
        let user = machine.find(ADD_LINES).expect("add_lines");
        assert!(!user.is_system());
        assert_eq!(user.from_states(), ["open"]);
        assert_eq!(user.to_state(), Some("open"));
        let system = machine.find(ADD_LINES_BY_SYSTEM).expect("by system");
        assert!(system.is_system());
        assert_eq!(
            user.guard_names(),
            ["location_scope", "item_in_scope", "quantity_moved_in"]
        );
    }

    #[test]
    fn a_hold_closes_by_itself_and_is_cancelled_by_a_decider() {
        let machine = StockHold::machine();
        let close = machine.find(CLOSE).expect("close");
        assert!(close.is_system());
        assert_eq!(close.from_states(), ["open"]);
        assert_eq!(close.to_state(), Some("closed"));
        assert_eq!(close.guard_names(), ["every_line_decided"]);
        let cancel = machine.find(CANCEL).expect("cancel");
        assert!(!cancel.is_system());
        assert_eq!(cancel.to_state(), Some("cancelled"));
        assert_eq!(cancel.guard_names(), ["not_a_recall", "no_line_decided"]);
        for state in ["closed", "cancelled"] {
            let ended = machine.states().iter().find(|s| s.name == state);
            assert!(ended.is_some_and(|s| s.terminal), "{state}");
        }
    }

    #[test]
    fn the_hold_numbers_follow_the_series() {
        assert_eq!(STOCK_HOLD_SERIES.prefix, "HLD");
        assert!(REASONS.contains(&"recall"));
    }
}
