//! The Stock hold (wl-inventory §3, §4 *Stock hold*, issue #230): stock is
//! blocked without moving it, whole lots (every location, the lot goes
//! `on_hold`) or the stock at one location (the lot status stays as it is;
//! only that balance is held). Held quantity is not available for picking,
//! issue, consumption, transfer or dispatch ([`crate::balance`]).
//!
//! ```text
//! [*] --place--> open --add_lines--> open
//! ```
//!
//! This file has the rows and the machine; `place`, the notices and the
//! system service are in [`crate::hold_service`].
//!
//! - `place` creates the hold open, with its lines (item, location, lot,
//!   quantity) made from the balances in scope.
//! - `add_lines` (a holder of `stock.hold`) and `add_lines_by_system` (a
//!   goods receipt's `post` or a transfer's `receive` into the location of
//!   an open hold on a location): the quantity moved in joins the hold as new
//!   lines with the hold's reason and reference.
//!
//! `close`, `cancel` and the decisions on the lines come with the issue that
//! decides them (BL-STK-12); the states are declared.
//!
//! Decisions: the reason is one of [`REASONS`]; a reference is required (§4
//! guard "reason and reference"); a hold on a location with no items named
//! covers every item there, else only the named ones; the lines of a second
//! hold on a lot that is `on_hold` already are added and the lot stays as it
//! is; the balance of an `expired` lot is a line too and the lot stays
//! `expired`.

use renox::db::Executor;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::STOCK_HOLD_SUBJECT_TYPE;
use wl_kernel::numbering::{Reset, Series};

use crate::permissions;
use crate::texts::english;

/// The user's `add_lines` transition's name.
pub const ADD_LINES: &str = "add_lines";

/// The system's `add_lines_by_system` transition's name.
pub const ADD_LINES_BY_SYSTEM: &str = "add_lines_by_system";

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
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockHold {
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

/// One held quantity: an item at a location, of a lot when it is tracked.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_stock_hold_lines", index(stock_hold_id))]
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
                    let mut row = StockHoldLine {
                        stock_hold_id: hold_id,
                        item_id: line.item_id,
                        location_id: line.location_id,
                        lot_id: line.lot_id,
                        quantity_milli: line.quantity_milli,
                        ..StockHoldLine::default()
                    };
                    row.insert(&mut *tx).await?;
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
    fn the_hold_numbers_follow_the_series() {
        assert_eq!(STOCK_HOLD_SERIES.prefix, "HLD");
        assert!(REASONS.contains(&"recall"));
    }
}
