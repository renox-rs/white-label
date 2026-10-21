//! The Transfer (wl-inventory §3, §4 *Transfer*): stock moves between two
//! locations in two steps.
//!
//! ```text
//! [*] --create--> draft --send--> in_transit --receive--> received
//!                   \--cancel--> cancelled
//! ```
//!
//! - `send` (`stock.transfer` at the source): refused unless the stock is
//!   available, the lots are `released` and none of the quantity is on hold;
//!   a `transfer_out` movement takes each line to the in-transit location at
//!   the item's average cost.
//! - `receive` (`stock.transfer` at the destination): a `transfer_in`
//!   movement puts each line from the in-transit location at the destination.
//!   What arrives joins an open hold on the destination and posts the pending
//!   issues waiting there ([`crate::arrival`]).
//! - `cancel` (`stock.transfer`): a draft that is not sent.
//!
//! Decisions: `receive` takes in every line as it was sent (entering a
//! smaller quantity, with the shortage posted as an `adjust` that needs
//! approval above the limit, comes with the transfer discrepancy issue); the
//! in-transit location is the one coded [`TRANSIT_CODE`], made the first time
//! a transfer is sent; a `send` reads what stands in its way in a transaction
//! of its own, rolled back, and the effect reads it again before it moves
//! anything.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::TRANSFER_SUBJECT_TYPE;
use wl_kernel::numbering::{Reset, Series};

use crate::balance;
use crate::item::{Item, quantity_text, value_of};
use crate::location::{Location, LocationType, scope_of};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::texts::said;

/// The `send` transition's name.
pub const SEND: &str = "send";

/// The `receive` transition's name.
pub const RECEIVE: &str = "receive";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The code of the location stock is held in while it is on its way.
pub const TRANSIT_CODE: &str = "IN-TRANSIT";

/// The numbers of transfers: `TRF-2026-0001`.
pub const TRANSFER_SERIES: Series = Series {
    key: "transfer",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "TRF",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a transfer stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransferStatus {
    /// Lines being prepared.
    #[default]
    Draft,
    /// Left the source; held in the in-transit location.
    InTransit,
    /// Received at the destination (terminal).
    Received,
    /// Not sent (terminal).
    Cancelled,
}

/// Stock moving from one location to another.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_transfers",
    state_machine,
    unique(number),
    index(status)
)]
pub struct Transfer {
    /// The primary key.
    pub id: i64,
    /// The number from the `TRF-` series.
    pub number: Option<String>,
    /// Where the stock leaves from.
    pub from_location_id: i64,
    /// Where the stock goes.
    pub to_location_id: i64,
    /// The type of the source location's scope.
    pub from_scope_type: Option<String>,
    /// The id of the source location's scope.
    pub from_scope_id: Option<i64>,
    /// The type of the destination's scope.
    pub to_scope_type: Option<String>,
    /// The id of the destination's scope.
    pub to_scope_id: Option<i64>,
    /// The in-transit location the stock is held in.
    pub transit_location_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: TransferStatus,
    /// The user who sent it.
    pub sent_by: Option<i64>,
    /// When it was sent.
    pub sent_at: Option<DateTime>,
    /// The user who received it.
    pub received_by: Option<i64>,
    /// When it was received.
    pub received_at: Option<DateTime>,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Transfer {
    /// The transfer as a [`Subject`]: `inventory.transfer:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(TRANSFER_SUBJECT_TYPE, self.id)
    }

    /// Who may send it: the scope of the source location.
    pub fn from_scope(&self) -> Scope {
        scope_of(self.from_scope_type.as_deref(), self.from_scope_id)
    }

    /// Who may receive it: the scope of the destination.
    pub fn to_scope(&self) -> Scope {
        scope_of(self.to_scope_type.as_deref(), self.to_scope_id)
    }
}

/// One line of a transfer. Quantities are thousandths of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_transfer_lines", unique(transfer_id, position))]
pub struct TransferLine {
    /// The primary key.
    pub id: i64,
    /// The transfer.
    pub transfer_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity.
    pub quantity_milli: i64,
    /// The average cost of one base unit when it was sent, in fils.
    pub unit_cost: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A line of a transfer to make.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewTransferLine {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

impl NewTransferLine {
    /// `quantity_milli` of `item_id`.
    pub fn of(item_id: i64, quantity_milli: i64) -> Self {
        NewTransferLine {
            item_id,
            lot_id: None,
            quantity_milli,
        }
    }

    /// Of lot `lot_id`.
    pub fn lot(mut self, lot_id: i64) -> Self {
        self.lot_id = Some(lot_id);
        self
    }
}

/// A transfer to prepare as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTransfer {
    /// Where the stock leaves from.
    pub from_location_id: i64,
    /// Where the stock goes.
    pub to_location_id: i64,
    /// The lines.
    pub lines: Vec<NewTransferLine>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Prepares `new` as a `draft` transfer by user `by`.
///
/// # Errors
///
/// 422 for the same location at both ends, a transfer without lines, a line
/// without quantity, an unknown item or a lot-tracked item without a lot.
pub async fn create_draft(db: &Db, by: Option<i64>, new: NewTransfer) -> Result<Transfer> {
    if new.from_location_id == new.to_location_id {
        return Err(refused(said("wl_inventory.transfer.same_location", &[])));
    }
    if new.lines.is_empty() {
        return Err(refused(said("wl_inventory.transfer.no_lines", &[])));
    }
    let from = Location::find_or_404(db, new.from_location_id).await?;
    let to = Location::find_or_404(db, new.to_location_id).await?;
    let mut rows = Vec::new();
    for (position, line) in (1..).zip(&new.lines) {
        let position_text = position.to_string();
        if line.quantity_milli <= 0 {
            return Err(refused(said(
                "wl_inventory.transfer.bad_line",
                &[("position", &position_text)],
            )));
        }
        let Some(item) = Item::find(db, line.item_id).await? else {
            return Err(refused(said(
                "wl_inventory.receipt.unknown_item",
                &[("position", &position_text)],
            )));
        };
        if item.lot_tracked && line.lot_id.is_none() {
            return Err(refused(said(
                "wl_inventory.transfer.lot_needed",
                &[("position", &position_text), ("item", &item.code)],
            )));
        }
        rows.push(TransferLine {
            position,
            item_id: item.id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
            ..TransferLine::default()
        });
    }
    let mut tx = db.begin().await?;
    let mut transfer = Transfer {
        number: Some(TRANSFER_SERIES.next(&mut tx, None, None, today()).await?),
        from_location_id: from.id,
        to_location_id: to.id,
        from_scope_type: from.scope_type.clone(),
        from_scope_id: from.scope_id,
        to_scope_type: to.scope_type.clone(),
        to_scope_id: to.scope_id,
        status: TransferStatus::Draft,
        created_by: by,
        ..Transfer::default()
    };
    transfer.insert(&mut tx).await?;
    for row in &mut rows {
        row.transfer_id = transfer.id;
        row.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(transfer)
}

/// The lines of transfer `transfer_id`, in order.
///
/// # Errors
///
/// A database error.
pub async fn lines_of<'c>(
    db: impl renox::db::Executor<'c>,
    transfer_id: i64,
) -> Result<Vec<TransferLine>> {
    TransferLine::query()
        .where_(TransferLine::TRANSFER_ID.eq(transfer_id))
        .order_by(TransferLine::POSITION)
        .get(db)
        .await
}

/// What a `send` run carries to its guard: the first thing that stands in the
/// way, `None` when nothing does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SendInput {
    /// The user who sends it.
    pub by: Option<i64>,
    /// The transfer has lines.
    pub lines_problem: Option<String>,
    /// A lot is not `released`.
    pub lot_problem: Option<String>,
    /// Part of the quantity is on hold, or not available.
    pub stock_problem: Option<String>,
}

impl SendInput {
    /// The first problem, in the order the guards run.
    pub fn first_problem(&self) -> Option<&str> {
        [&self.lines_problem, &self.lot_problem, &self.stock_problem]
            .into_iter()
            .find_map(|problem| problem.as_deref())
    }
}

/// What stands in the way of sending `transfer`, read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn gather_send(
    tx: &mut Transaction,
    transfer: &Transfer,
    by: Option<i64>,
) -> Result<SendInput> {
    let lines = lines_of(&mut *tx, transfer.id).await?;
    let mut found = SendInput {
        by,
        ..SendInput::default()
    };
    if lines.is_empty() {
        found.lines_problem = Some(said("wl_inventory.transfer.no_lines", &[]));
        return Ok(found);
    }
    let mut claimed: std::collections::BTreeMap<(i64, Option<i64>), i64> =
        std::collections::BTreeMap::new();
    for line in &lines {
        let position = line.position.to_string();
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        if let Some(lot_id) = line.lot_id {
            let lot = Lot::find_or_404(&mut *tx, lot_id).await?;
            if lot.status != LotStatus::Released && found.lot_problem.is_none() {
                found.lot_problem = Some(said(
                    "wl_inventory.transfer.not_released",
                    &[("position", &position), ("lot", &lot.number)],
                ));
            }
        }
        let needed = claimed.entry((line.item_id, line.lot_id)).or_insert(0);
        *needed += line.quantity_milli;
        let needed = *needed;
        let found_balance = balance::balance(
            &mut *tx,
            line.item_id,
            transfer.from_location_id,
            line.lot_id,
        )
        .await?;
        if found_balance.available_milli() >= needed || found.stock_problem.is_some() {
            continue;
        }
        let without_hold = found_balance.on_hand_milli - found_balance.reserved_milli;
        found.stock_problem = Some(if without_hold >= needed {
            said(
                "wl_inventory.transfer.held",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("held", &quantity_text(found_balance.held_milli)),
                ],
            )
        } else {
            said(
                "wl_inventory.transfer.not_available",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("available", &quantity_text(found_balance.available_milli())),
                    ("quantity", &quantity_text(needed)),
                ],
            )
        });
    }
    Ok(found)
}

fn send_input(ctx: &TransitionCtx) -> Option<SendInput> {
    ctx.input_as::<SendInput>().ok()
}

/// A `send` guard that refuses with the problem `pick` finds in the input.
fn with_problem(
    transition: Transition<Transfer>,
    name: &'static str,
    pick: fn(&SendInput) -> Option<String>,
) -> Transition<Transfer> {
    transition.guard(name, move |_transfer: &Transfer, ctx| {
        let problem = send_input(ctx).map_or_else(
            || Some(said("wl_inventory.transfer.not_draft", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// The in-transit location, made when there is none.
async fn transit_location(tx: &mut Transaction) -> Result<i64> {
    if let Some(found) = Location::query()
        .where_(Location::CODE.eq(TRANSIT_CODE))
        .first(&mut *tx)
        .await?
    {
        return Ok(found.id);
    }
    let mut location = Location {
        code: TRANSIT_CODE.to_owned(),
        name_en: "In transit".to_owned(),
        location_type: LocationType::InTransit,
        ..Location::default()
    };
    location.insert(&mut *tx).await?;
    Ok(location.id)
}

/// The effect of `send`: the stock leaves the source for the in-transit
/// location.
async fn send_in(tx: &mut Transaction, transfer: Transfer, by: Option<i64>) -> Result {
    // Stock may have moved or been held since the guards looked.
    let found = gather_send(tx, &transfer, by).await?;
    if let Some(problem) = found.first_problem() {
        return Err(refused(problem));
    }
    let now = renox::db::now();
    let transit = transit_location(tx).await?;
    let source = transfer.subject();
    for line in lines_of(&mut *tx, transfer.id).await? {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let value = value_of(line.quantity_milli, item.average_cost);
        let mut moved = StockMovement::moving(
            MovementType::TransferOut,
            item.id,
            line.lot_id,
            Some(transfer.from_location_id),
            Some(transit),
            line.quantity_milli,
            item.average_cost,
            value,
            &source,
            by,
        );
        moved.moved_at = now;
        moved.insert(&mut *tx).await?;
        TransferLine::query()
            .where_(TransferLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[("unit_cost", &item.average_cost), ("updated_at", &now)],
            )
            .await?;
    }
    Transfer::query()
        .where_(Transfer::ID.eq(transfer.id))
        .update(
            &mut *tx,
            &[
                ("transit_location_id", &Some(transit)),
                ("sent_by", &by),
                ("sent_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The effect of `receive`: the stock goes from the in-transit location to
/// the destination.
async fn receive_in(tx: &mut Transaction, transfer: Transfer, by: Option<i64>) -> Result {
    let now = renox::db::now();
    let Some(transit) = transfer.transit_location_id else {
        return Err(refused(said("wl_inventory.transfer.not_in_transit", &[])));
    };
    let source = transfer.subject();
    for line in lines_of(&mut *tx, transfer.id).await? {
        let value = value_of(line.quantity_milli, line.unit_cost);
        let mut moved = StockMovement::moving(
            MovementType::TransferIn,
            line.item_id,
            line.lot_id,
            Some(transit),
            Some(transfer.to_location_id),
            line.quantity_milli,
            line.unit_cost,
            value,
            &source,
            by,
        );
        moved.moved_at = now;
        moved.insert(&mut *tx).await?;
    }
    Transfer::query()
        .where_(Transfer::ID.eq(transfer.id))
        .update(
            &mut *tx,
            &[
                ("received_by", &by),
                ("received_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for Transfer {
    type State = TransferStatus;
    const COLUMN: renox::db::Col<Transfer, TransferStatus> = Transfer::STATUS;

    fn machine() -> Machine<Self> {
        use TransferStatus::{Cancelled, Draft, InTransit, Received};
        let send = Transition::new(SEND)
            .label("Send")
            .tone("primary")
            .permission_in(permissions::STOCK_TRANSFER, |transfer: &Transfer| {
                transfer.from_scope()
            })
            .from([Draft])
            .to(InTransit);
        let send = with_problem(send, "has_lines", |i| i.lines_problem.clone());
        let send = with_problem(send, "released_lots_only", |i| i.lot_problem.clone());
        let send = with_problem(send, "stock_available_and_not_held", |i| {
            i.stock_problem.clone()
        });
        let send = send.effect(|transfer: &Transfer, ctx, tx| {
            let transfer = transfer.clone();
            let by = send_input(ctx).and_then(|input| input.by);
            Box::pin(async move { send_in(tx, transfer, by).await })
        });
        let receive = Transition::new(RECEIVE)
            .label("Receive")
            .tone("primary")
            .permission_in(permissions::STOCK_TRANSFER, |transfer: &Transfer| {
                transfer.to_scope()
            })
            .from([InTransit])
            .to(Received)
            .effect(|transfer: &Transfer, ctx, tx| {
                let transfer = transfer.clone();
                let by = ctx.actor().map(|user| user.id);
                Box::pin(async move { receive_in(tx, transfer, by).await })
            });
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(InTransit)
            .tone("info")
            .state(Received)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(send)
            .transition(receive)
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission(permissions::STOCK_TRANSFER)
                    .from([Draft])
                    .to(Cancelled),
            )
    }
}

/// `user` sends transfer `id`. Refused with the first problem before
/// anything moves.
///
/// # Errors
///
/// `Forbidden` without `stock.transfer` at the source; `NotFrom` unless
/// `draft`; `Guard` (422) when the stock is not available, a lot is not
/// released or part of the quantity is on hold.
pub async fn send(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Transfer, TransitionError> {
    let transfer = Transfer::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    let found = gather_send(&mut tx, &transfer, Some(user.id)).await?;
    tx.rollback().await?;
    if transfer.status == TransferStatus::Draft
        && let Some(problem) = found.first_problem()
    {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    let _outcome = transfer
        .transition(SEND)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    Ok(Transfer::find_or_404(&state.db, id).await?)
}

/// `user` receives transfer `id` at the destination.
///
/// # Errors
///
/// `Forbidden` without `stock.transfer` at the destination; `NotFrom` unless
/// `in_transit`.
pub async fn receive(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Transfer, TransitionError> {
    let transfer = Transfer::find_or_404(&state.db, id).await?;
    let _outcome = transfer.transition(RECEIVE).by(user).run(state).await?;
    Ok(Transfer::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transfer_is_sent_then_received_and_a_draft_can_be_cancelled() {
        let machine = Transfer::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let send = machine.find(SEND).expect("send");
        assert_eq!(send.from_states(), ["draft"]);
        assert_eq!(send.to_state(), Some("in_transit"));
        assert_eq!(
            send.guard_names(),
            [
                "has_lines",
                "released_lots_only",
                "stock_available_and_not_held"
            ]
        );
        let receive = machine.find(RECEIVE).expect("receive");
        assert_eq!(receive.from_states(), ["in_transit"]);
        assert_eq!(receive.to_state(), Some("received"));
        let cancel = machine.find(CANCEL).expect("cancel");
        assert_eq!(cancel.to_state(), Some("cancelled"));
        for name in [SEND, RECEIVE, CANCEL] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
    }

    #[test]
    fn the_first_problem_follows_the_order_of_the_guards() {
        let mut input = SendInput {
            stock_problem: Some("stock".into()),
            ..SendInput::default()
        };
        assert_eq!(input.first_problem(), Some("stock"));
        input.lot_problem = Some("lot".into());
        assert_eq!(input.first_problem(), Some("lot"));
        assert_eq!(SendInput::default().first_problem(), None);
    }

    #[test]
    fn the_numbers_follow_the_series() {
        assert_eq!(TRANSFER_SERIES.prefix, "TRF");
    }
}
