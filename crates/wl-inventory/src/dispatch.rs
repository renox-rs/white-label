//! The Dispatch (wl-inventory §3, §4 *Dispatch*, §5.4): goods picked, packed
//! and sent to a recipient for a source such as a sales order. The lifecycle:
//!
//! ```text
//! [*] --create--> draft --pick--> picked --dispatch--> dispatched --confirm_delivery--> delivered
//!                   \--cancel--> cancelled   \--cancel--> cancelled
//! dispatched --mark_lost--> lost --found--> dispatched
//! delivered --mark_lost--> lost        delivered --reverse--> dispatched
//! ```
//!
//! What is built here, with the issue that asked for it:
//!
//! | Part | Module | Issue |
//! |---|---|---|
//! | the entity, the lines, the draft, the machine | this file | base of #240 to #244 |
//! | `pick` (released lots, no held quantity, stock reserved), `cancel`, re-picking a line | [`crate::dispatch_pick`] | #240 |
//! | `dispatch` (no lot on hold, the credit re-check, the hand-over, the movements, the delivery note) | [`crate::dispatch_send`] | #240 |
//! | `confirm_delivery` per line and the refused goods received back into quarantine | [`crate::dispatch_deliver`] | #241 |
//! | `mark_lost` and `found` (the loss pending the claim) | [`crate::dispatch_loss`] | #243 |
//! | `reverse` (a delivery reversal reopens the dispatch) | [`crate::dispatch_reverse`] | #244 |
//! | the kernel events and the listeners of `wl-delivery`'s events | [`crate::dispatch_events`] | all |
//!
//! Not built here: `return` (comes with the dispatch return issue) and
//! `discard` (BL-STK-23, #242, which needs `DeliveryDiscarded` of BL-DLV-07).
//! The states `returned` and `discarded` are declared so those issues add only
//! their transitions.
//!
//! *Reserved* stock: the lines of the dispatches that are `picked` at their
//! location count as reserved in [`crate::balance::reserved_at`]; a dispatch
//! that leaves, is cancelled or has a line re-picked gives its reservation
//! back with no other bookkeeping.
//!
//! Decision: a draft is made with [`create_draft`] by the module that sells
//! (a sales order's dispatch), as the lines of a transfer are made; a lot-tracked
//! item names its lot in the draft and `pick` checks it (FEFO is suggested when
//! a line is re-picked, [`crate::dispatch_pick::repick`]).

use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::{DISPATCH_LINE_SUBJECT_TYPE, DISPATCH_SUBJECT_TYPE};
use wl_kernel::numbering::{Reset, Series};

use crate::item::Item;
use crate::location::{Location, scope_of};
use crate::texts::said;

/// The numbers of dispatches: `DSP-2026-0001`.
pub const DISPATCH_SERIES: Series = Series {
    key: "dispatch",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "DSP",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// Where a dispatch stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DispatchStatus {
    /// Lines and lots being chosen.
    #[default]
    Draft,
    /// Picked and packed; stock reserved.
    Picked,
    /// Left with the delivery note; on a delivery run or with a courier.
    Dispatched,
    /// Proof of delivery received; refused lines are *in return* until
    /// received back into quarantine. Not terminal.
    Delivered,
    /// Refused or returned (terminal).
    Returned,
    /// Destroyed by wl-delivery after a failed attempt (terminal).
    Discarded,
    /// wl-delivery marked the delivery lost; the value is held as a loss
    /// while the claim runs. Not terminal.
    Lost,
    /// Not shipped (terminal).
    Cancelled,
}

/// How the goods are handed over (§4 `dispatch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handover {
    /// Our own delivery run (wl-delivery).
    OwnDelivery,
    /// A courier, with a tracking number.
    Courier,
    /// The customer collects, and the collector signs (`DOC.SIG.01`).
    Collection,
}

impl Handover {
    /// The name stored on the dispatch.
    pub fn as_str(self) -> &'static str {
        match self {
            Handover::OwnDelivery => "own_delivery",
            Handover::Courier => "courier",
            Handover::Collection => "collection",
        }
    }

    /// The hand-over named `kind`.
    pub fn parse(kind: &str) -> Option<Handover> {
        match kind.trim() {
            "own_delivery" => Some(Handover::OwnDelivery),
            "courier" => Some(Handover::Courier),
            "collection" => Some(Handover::Collection),
            _ => None,
        }
    }
}

/// Goods sent to a recipient.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_dispatches",
    state_machine,
    unique(number),
    index(status),
    index(source_type, source_id)
)]
pub struct Dispatch {
    /// The primary key.
    pub id: i64,
    /// The number from the `DSP-` series.
    pub number: Option<String>,
    /// The type of the record the dispatch fulfils.
    pub source_type: String,
    /// The id of the record the dispatch fulfils.
    pub source_id: i64,
    /// The recipient, when it is a party.
    pub recipient_party_id: Option<i64>,
    /// The recipient's name, as the delivery and the note say it.
    pub recipient_name: String,
    /// Where the goods go.
    pub address: String,
    /// The location the goods leave from.
    pub location_id: i64,
    /// The type of the location's scope.
    pub scope_type: Option<String>,
    /// The id of the location's scope.
    pub scope_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DispatchStatus,
    /// `own_delivery`, `courier` or `collection`, once dispatched.
    pub handover: Option<String>,
    /// The courier's tracking number.
    pub tracking_number: Option<String>,
    /// The collector's signature (a document reference).
    pub collector_signature: Option<String>,
    /// The delivery wl-delivery was asked for (own delivery).
    pub delivery_id: Option<i64>,
    /// The delivery note, a PDF document.
    pub delivery_note: Option<String>,
    /// The user who picked it.
    pub picked_by: Option<i64>,
    /// When it was picked.
    pub picked_at: Option<DateTime>,
    /// The user who dispatched it.
    pub dispatched_by: Option<i64>,
    /// When it was dispatched.
    pub dispatched_at: Option<DateTime>,
    /// When the delivery was confirmed.
    pub delivered_at: Option<DateTime>,
    /// The proof of delivery that confirmed it: one confirmation per proof.
    pub proof_id: Option<i64>,
    /// Whether a delivery reversal marked that proof reversed.
    pub proof_reversed: bool,
    /// The status it had when it was marked lost.
    pub lost_from: Option<String>,
    /// The value held as a loss while the claim runs, in fils.
    pub loss_fils: i64,
    /// How many times a loss was held.
    pub loss_count: i64,
    /// Whether the loss is held now.
    pub loss_open: bool,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Dispatch {
    /// The dispatch as a [`Subject`]: `inventory.dispatch:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(DISPATCH_SUBJECT_TYPE, self.id)
    }

    /// The record the dispatch fulfils.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// Who may pick and dispatch it: the scope of its location.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }
}

/// One line of a dispatch. Quantities are thousandths of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_dispatch_lines", unique(dispatch_id, position))]
pub struct DispatchLine {
    /// The primary key.
    pub id: i64,
    /// The dispatch.
    pub dispatch_id: i64,
    /// The order of the line, from 1.
    pub position: i64,
    /// The item.
    pub item_id: i64,
    /// The lot the line is picked from.
    pub lot_id: Option<i64>,
    /// The quantity sent.
    pub quantity_milli: i64,
    /// The cost of one base unit when the goods left stock, in fils.
    pub unit_cost: i64,
    /// The type of the source line this line fulfils.
    pub source_line_type: Option<String>,
    /// The id of the source line this line fulfils.
    pub source_line_id: Option<i64>,
    /// The quantity the recipient accepted.
    pub delivered_milli: i64,
    /// The quantity the recipient refused.
    pub refused_milli: i64,
    /// The refusal reason's code.
    pub refusal_reason: Option<String>,
    /// Of the refused quantity, what was received back into quarantine.
    pub returned_milli: i64,
    /// Of the refused quantity, what was reported missing on return.
    pub missing_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DispatchLine {
    /// The line as a [`Subject`]: `inventory.dispatch_line:4`.
    pub fn subject(&self) -> Subject {
        Subject::new(DISPATCH_LINE_SUBJECT_TYPE, self.id)
    }

    /// The line of the source this line fulfils.
    pub fn source_line(&self) -> Option<Subject> {
        match (&self.source_line_type, self.source_line_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The refused quantity still *in return*: neither received back nor
    /// reported missing.
    pub fn in_return_milli(&self) -> i64 {
        (self.refused_milli - self.returned_milli - self.missing_milli).max(0)
    }

    /// The quantity that left and was not refused: what a loss is worth, and
    /// what the recipient may accept.
    pub fn accepted_candidate_milli(&self) -> i64 {
        (self.quantity_milli - self.refused_milli).max(0)
    }
}

/// A line of a dispatch to make.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewDispatchLine {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The line of the source this line fulfils.
    pub source_line: Option<Subject>,
}

impl NewDispatchLine {
    /// `quantity_milli` of `item_id`.
    pub fn of(item_id: i64, quantity_milli: i64) -> Self {
        NewDispatchLine {
            item_id,
            quantity_milli,
            ..NewDispatchLine::default()
        }
    }

    /// Of lot `lot_id`.
    pub fn lot(mut self, lot_id: i64) -> Self {
        self.lot_id = Some(lot_id);
        self
    }

    /// Fulfilling `line` of the source.
    pub fn for_line(mut self, line: Subject) -> Self {
        self.source_line = Some(line);
        self
    }
}

/// A dispatch to prepare as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDispatch {
    /// The record it fulfils (a sales order).
    pub source: Subject,
    /// The recipient, when it is a party.
    pub recipient: Option<wl_kernel::PartyId>,
    /// The recipient's name.
    pub recipient_name: String,
    /// Where the goods go.
    pub address: String,
    /// The location the goods leave from.
    pub location_id: i64,
    /// The lines.
    pub lines: Vec<NewDispatchLine>,
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Prepares `new` as a `draft` dispatch by user `by`.
///
/// # Errors
///
/// 422 for a dispatch without lines, a line without quantity, an unknown item
/// or a lot-tracked item without a lot.
pub async fn create_draft(db: &Db, by: Option<i64>, new: NewDispatch) -> Result<Dispatch> {
    if new.lines.is_empty() {
        return Err(refused(said("wl_inventory.dispatch.no_lines", &[])));
    }
    let location = Location::find_or_404(db, new.location_id).await?;
    let mut rows = Vec::new();
    for (position, line) in (1..).zip(&new.lines) {
        let position_text = position.to_string();
        if line.quantity_milli <= 0 {
            return Err(refused(said(
                "wl_inventory.dispatch.bad_line",
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
                "wl_inventory.dispatch.lot_needed",
                &[("position", &position_text), ("item", &item.code)],
            )));
        }
        rows.push(DispatchLine {
            position,
            item_id: item.id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
            source_line_type: line.source_line.as_ref().map(|s| s.subject_type.clone()),
            source_line_id: line.source_line.as_ref().map(|s| s.subject_id),
            ..DispatchLine::default()
        });
    }
    let mut tx = db.begin().await?;
    let mut dispatch = Dispatch {
        number: Some(DISPATCH_SERIES.next(&mut tx, None, None, today()).await?),
        source_type: new.source.subject_type.clone(),
        source_id: new.source.subject_id,
        recipient_party_id: new.recipient.map(|party| party.0),
        recipient_name: new.recipient_name.trim().to_owned(),
        address: new.address.trim().to_owned(),
        location_id: location.id,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        status: DispatchStatus::Draft,
        created_by: by,
        ..Dispatch::default()
    };
    dispatch.insert(&mut tx).await?;
    for row in &mut rows {
        row.dispatch_id = dispatch.id;
        row.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(dispatch)
}

/// The lines of dispatch `dispatch_id`, in order.
///
/// # Errors
///
/// A database error.
pub async fn lines_of<'c>(
    db: impl renox::db::Executor<'c>,
    dispatch_id: i64,
) -> Result<Vec<DispatchLine>> {
    DispatchLine::query()
        .where_(DispatchLine::DISPATCH_ID.eq(dispatch_id))
        .order_by(DispatchLine::POSITION)
        .get(db)
        .await
}

/// The dispatches made for `source`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<Dispatch>> {
    Dispatch::query()
        .where_(Dispatch::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(Dispatch::SOURCE_ID.eq(source.subject_id))
        .order_by(Dispatch::ID)
        .get(db)
        .await
}

/// The dispatch whose delivery is `source` (a `wl-delivery` event names the
/// record the delivery was requested for), `None` for another kind of source.
///
/// # Errors
///
/// A database error.
pub async fn of_delivery_source(db: &Db, source: &Subject) -> Result<Option<Dispatch>> {
    if source.subject_type != DISPATCH_SUBJECT_TYPE {
        return Ok(None);
    }
    Dispatch::find(db, source.subject_id).await
}

/// What a dispatch line reserves: the quantity of `item_id` that the picked
/// dispatches at `location_id` hold aside (of `lot_id` when given).
///
/// # Errors
///
/// A database error.
pub async fn reserved_picked<'c>(
    db: impl renox::db::Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    let mut conn = db.into_conn();
    let picked = Dispatch::query()
        .where_(Dispatch::STATUS.eq(DispatchStatus::Picked))
        .where_(Dispatch::LOCATION_ID.eq(location_id))
        .get(conn.reborrow())
        .await?;
    let mut reserved = 0;
    for dispatch in picked {
        let lines = DispatchLine::query()
            .where_(DispatchLine::DISPATCH_ID.eq(dispatch.id))
            .where_(DispatchLine::ITEM_ID.eq(item_id))
            .get(conn.reborrow())
            .await?;
        reserved += lines
            .iter()
            .filter(|line| lot_id.is_none() || line.lot_id == lot_id)
            .map(|line| line.quantity_milli)
            .sum::<i64>();
    }
    Ok(reserved)
}

impl StateMachine for Dispatch {
    type State = DispatchStatus;
    const COLUMN: renox::db::Col<Dispatch, DispatchStatus> = Dispatch::STATUS;

    fn machine() -> Machine<Self> {
        use DispatchStatus::{
            Cancelled, Delivered, Discarded, Dispatched, Draft, Lost, Picked, Returned,
        };
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Picked)
            .tone("info")
            .state(Dispatched)
            .tone("info")
            .state(Delivered)
            .tone("success")
            .state(Returned)
            .tone("error")
            .terminal()
            .state(Discarded)
            .tone("error")
            .terminal()
            .state(Lost)
            .tone("error")
            .state(Cancelled)
            .tone("neutral")
            .terminal();
        let machine = crate::dispatch_pick::add_to(machine);
        let machine = crate::dispatch_send::add_to(machine);
        let machine = crate::dispatch_deliver::add_to(machine);
        let machine = crate::dispatch_loss::add_to(machine);
        crate::dispatch_reverse::add_to(machine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dispatch_starts_as_a_draft_with_the_eight_states() {
        let machine = Dispatch::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(machine.states().len(), 8);
        for terminal in ["returned", "discarded", "cancelled"] {
            let state = machine.states().iter().find(|s| s.name == terminal);
            assert!(state.is_some_and(|s| s.terminal), "{terminal}");
        }
        for open in ["delivered", "lost"] {
            let state = machine.states().iter().find(|s| s.name == open);
            assert!(state.is_some_and(|s| !s.terminal), "{open}");
        }
    }

    #[test]
    fn a_handover_is_stored_by_name() {
        for handover in [
            Handover::OwnDelivery,
            Handover::Courier,
            Handover::Collection,
        ] {
            assert_eq!(Handover::parse(handover.as_str()), Some(handover));
        }
        assert_eq!(Handover::parse("pigeon"), None);
    }

    #[test]
    fn the_refused_quantity_is_in_return_until_received_or_missing() {
        let line = DispatchLine {
            quantity_milli: 10_000,
            refused_milli: 2_000,
            returned_milli: 1_000,
            missing_milli: 500,
            ..DispatchLine::default()
        };
        assert_eq!(line.in_return_milli(), 500);
        assert_eq!(line.accepted_candidate_milli(), 8_000);
    }
}
