//! The Dispatch (wl-inventory §3, §4 *Dispatch*): goods picked, packed and
//! shipped to a recipient for a source such as a sales order. The base that
//! BL-STK-26 (#245) acts on; the move to the customer is wl-delivery's.
//!
//! ```text
//! [*] --create--> draft --pick--> picked --dispatch--> dispatched --> delivered | returned | discarded | lost
//!                   \--cancel--/      \--cancel--> cancelled
//! ```
//!
//! Built here as far as #245 needs: [`create_draft`], `pick` (stock reserved:
//! a picked dispatch's lines reduce what is available, see
//! [`crate::balance::reserved_at`]), `cancel` (`dispatch.manage`) and
//! `cancel_by_system` (a sales order line cancelled with the outcome *return
//! to stock*, [`crate::sales_cancel`]). The states after `picked` are declared
//! but their transitions (`dispatch`, `confirm_delivery`, `return`, `discard`,
//! `mark_lost`, `found`, `reverse`; BL-STK-21 to 25 and Epic 07) are not built;
//! a test or the 07 base puts a dispatch in `dispatched` with
//! `insert_in_state`.
//!
//! Decisions: the source of a dispatch is a record such as `sales.sales_order:7`
//! and each line may name the sale line it fulfils (`sale_line_id`), which is
//! how a cancelled line finds its dispatch; the scope for permissions is that
//! of the location the goods are picked at; `pick` is refused with the first
//! line that is not available or is on hold.

use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::state_machine::{
    Machine, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::balance;
use crate::item::{Item, quantity_text};
use crate::location::{Location, scope_of};
use crate::permissions;
use crate::texts::said;

/// The subject type of a dispatch.
pub const DISPATCH_SUBJECT_TYPE: &str = "inventory.dispatch";

/// The `pick` transition's name.
pub const PICK: &str = "pick";

/// The user's `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// The system's `cancel_by_system` transition's name.
pub const CANCEL_BY_SYSTEM: &str = "cancel_by_system";

/// Where a dispatch stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DispatchStatus {
    /// Lines and lots being chosen.
    #[default]
    Draft,
    /// Picked and packed; stock reserved.
    Picked,
    /// Left with the delivery note.
    Dispatched,
    /// Proof of delivery received.
    Delivered,
    /// Refused or returned (terminal).
    Returned,
    /// Perishable goods destroyed after a failed attempt (terminal).
    Discarded,
    /// The delivery was lost.
    Lost,
    /// Not shipped (terminal).
    Cancelled,
}

impl DispatchStatus {
    /// Whether the goods are still in the store (draft or picked).
    pub fn in_store(self) -> bool {
        matches!(self, DispatchStatus::Draft | DispatchStatus::Picked)
    }
}

/// Goods shipped to a recipient.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_dispatches",
    state_machine,
    index(status),
    index(source_type, source_id)
)]
pub struct Dispatch {
    /// The primary key.
    pub id: i64,
    /// The location the goods are picked at.
    pub location_id: i64,
    /// The type of the location's scope.
    pub scope_type: Option<String>,
    /// The id of the location's scope.
    pub scope_id: Option<i64>,
    /// The type of the record it fulfils (a sales order).
    pub source_type: String,
    /// The id of the record it fulfils.
    pub source_id: i64,
    /// The recipient party.
    pub recipient_party_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DispatchStatus,
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

    /// The record it fulfils.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// Who may manage it: the scope of its location.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }

    /// Its number as shown: `DSP-0007`.
    pub fn number(&self) -> String {
        format!("DSP-{:04}", self.id)
    }
}

/// One line of a dispatch. Quantities are thousandths of the base unit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_dispatch_lines", index(dispatch_id), index(sale_line_id))]
pub struct DispatchLine {
    /// The primary key.
    pub id: i64,
    /// The dispatch.
    pub dispatch_id: i64,
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The quantity.
    pub quantity_milli: i64,
    /// The sale line (in `wl-sales`) it fulfils.
    pub sale_line_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
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
    /// The sale line it fulfils.
    pub sale_line_id: Option<i64>,
}

impl NewDispatchLine {
    /// `quantity_milli` of `item_id` for the sale line `sale_line_id`.
    pub fn of(item_id: i64, quantity_milli: i64, sale_line_id: i64) -> Self {
        NewDispatchLine {
            item_id,
            lot_id: None,
            quantity_milli,
            sale_line_id: Some(sale_line_id),
        }
    }

    /// Of lot `lot_id`.
    pub fn lot(mut self, lot_id: i64) -> Self {
        self.lot_id = Some(lot_id);
        self
    }
}

/// A dispatch to prepare as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDispatch {
    /// The location the goods are picked at.
    pub location_id: i64,
    /// The record it fulfils.
    pub source: Subject,
    /// The recipient party.
    pub recipient_party_id: Option<i64>,
    /// The lines.
    pub lines: Vec<NewDispatchLine>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Prepares `new` as a `draft` dispatch by user `by`.
///
/// # Errors
///
/// 422 for a dispatch without lines, a line without quantity or an unknown
/// item.
pub async fn create_draft(db: &Db, by: Option<i64>, new: NewDispatch) -> Result<Dispatch> {
    if new.lines.is_empty() {
        return Err(refused(said("wl_inventory.dispatch.no_lines", &[])));
    }
    if new.lines.iter().any(|line| line.quantity_milli <= 0) {
        return Err(refused(said("wl_inventory.dispatch.bad_line", &[])));
    }
    let location = Location::find_or_404(db, new.location_id).await?;
    for line in &new.lines {
        if Item::find(db, line.item_id).await?.is_none() {
            return Err(refused(said("wl_inventory.dispatch.bad_line", &[])));
        }
    }
    let mut tx = db.begin().await?;
    let mut dispatch = Dispatch {
        location_id: location.id,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        source_type: new.source.subject_type.clone(),
        source_id: new.source.subject_id,
        recipient_party_id: new.recipient_party_id,
        status: DispatchStatus::Draft,
        created_by: by,
        ..Dispatch::default()
    };
    dispatch.insert(&mut tx).await?;
    for line in &new.lines {
        let mut row = DispatchLine {
            dispatch_id: dispatch.id,
            item_id: line.item_id,
            lot_id: line.lot_id,
            quantity_milli: line.quantity_milli,
            sale_line_id: line.sale_line_id,
            ..DispatchLine::default()
        };
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
        .order_by(DispatchLine::ID)
        .get(db)
        .await
}

/// The dispatches that fulfil `source`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<Dispatch>> {
    Dispatch::query()
        .where_(Dispatch::SOURCE_TYPE.eq(source.subject_type.clone()))
        .where_(Dispatch::SOURCE_ID.eq(source.subject_id))
        .order_by(Dispatch::ID)
        .get(db)
        .await
}

/// Of the sale lines `lines` of `source`, those with goods picked or
/// dispatched: fulfilment has started (BL-SAL-05, BL-SAL-08). A vertical's
/// `FulfilmentProgress` adds this to what its own kitchen or workshop has
/// started.
///
/// # Errors
///
/// A database error.
pub async fn started_lines(db: &Db, source: &Subject, lines: &[i64]) -> Result<Vec<i64>> {
    let mut started = Vec::new();
    for dispatch in of_source(db, source).await? {
        if matches!(
            dispatch.status,
            DispatchStatus::Draft | DispatchStatus::Cancelled
        ) {
            continue;
        }
        for line in lines_of(db, dispatch.id).await? {
            if let Some(sale_line) = line.sale_line_id
                && lines.contains(&sale_line)
                && !started.contains(&sale_line)
            {
                started.push(sale_line);
            }
        }
    }
    Ok(started)
}

/// What a `pick` run carries to its guard: the first thing that stands in
/// the way, `None` when nothing does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PickInput {
    /// The dispatch has lines.
    pub lines_problem: Option<String>,
    /// A line is not available or part of it is on hold.
    pub stock_problem: Option<String>,
}

fn pick_input(ctx: &TransitionCtx) -> Option<PickInput> {
    ctx.input_as::<PickInput>().ok()
}

/// What stands in the way of picking `dispatch`.
async fn pick_problems(db: &Db, dispatch: &Dispatch) -> Result<PickInput> {
    let lines = lines_of(db, dispatch.id).await?;
    let mut input = PickInput::default();
    if lines.is_empty() {
        input.lines_problem = Some(said("wl_inventory.dispatch.no_lines", &[]));
        return Ok(input);
    }
    for line in &lines {
        let item = Item::find_or_404(db, line.item_id).await?;
        let found = balance::balance(db, line.item_id, dispatch.location_id, line.lot_id).await?;
        if found.available_milli() < line.quantity_milli {
            input.stock_problem = Some(if found.held_milli > 0 {
                said(
                    "wl_inventory.issue.held",
                    &[
                        ("item", &item.code),
                        ("held", &quantity_text(found.held_milli)),
                    ],
                )
            } else {
                said(
                    "wl_inventory.issue.not_available",
                    &[
                        ("item", &item.code),
                        ("available", &quantity_text(found.available_milli())),
                        ("quantity", &quantity_text(line.quantity_milli)),
                    ],
                )
            });
            break;
        }
    }
    Ok(input)
}

fn cancel_rules(transition: Transition<Dispatch>) -> Transition<Dispatch> {
    transition
        .from([DispatchStatus::Draft, DispatchStatus::Picked])
        .to(DispatchStatus::Cancelled)
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let id = dispatch.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                Dispatch::query()
                    .where_(Dispatch::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("cancel_reason", &reason),
                            ("updated_at", &renox::db::now()),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

impl StateMachine for Dispatch {
    type State = DispatchStatus;
    const COLUMN: renox::db::Col<Dispatch, DispatchStatus> = Dispatch::STATUS;

    fn machine() -> Machine<Self> {
        use DispatchStatus::{
            Cancelled, Delivered, Discarded, Dispatched, Draft, Lost, Picked, Returned,
        };
        let pick = Transition::new(PICK)
            .label("Pick")
            .tone("primary")
            .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
                dispatch.scope()
            })
            .from([Draft])
            .to(Picked)
            .guard("has_lines", |_dispatch: &Dispatch, ctx| {
                let problem = match pick_input(ctx) {
                    None => Some(said("wl_inventory.dispatch.not_draft", &[])),
                    Some(input) => input.lines_problem,
                };
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            })
            .guard("stock_available_and_not_held", |_dispatch: &Dispatch, ctx| {
                let problem = pick_input(ctx).and_then(|input| input.stock_problem);
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            });
        Machine::new()
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
            .terminal()
            .transition(pick)
            .transition(cancel_rules(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
                        dispatch.scope()
                    })
                    .reason_required(),
            ))
            .transition(cancel_rules(
                Transition::new(CANCEL_BY_SYSTEM)
                    .label("Cancel")
                    .system(),
            ))
    }
}

/// Picks `dispatch` as `user` (`dispatch.manage` at its location): its lines
/// must be available and none of the quantity on hold; the stock is then
/// reserved.
///
/// # Errors
///
/// `Forbidden` without the permission; `NotFrom` unless `draft`; `Guard` (422)
/// for a dispatch without lines or stock that is short or held.
pub async fn pick(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let input = pick_problems(&state.db, &dispatch).await?;
    let _outcome = dispatch
        .transition(PICK)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// Cancels `dispatch` as `user` with `reason`; the reservation is released.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` at its location; `NotFrom` unless
/// `draft` or `picked`; `Guard` (422) without a reason.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let _outcome = dispatch
        .transition(CANCEL)
        .by(user)
        .reason(reason)
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// Cancels the draft or picked `dispatch` as the system, for `reason`.
///
/// # Errors
///
/// `NotFrom` unless `draft` or `picked`.
pub async fn cancel_by_system(
    state: &AppState,
    id: i64,
    reason: &str,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let _outcome = dispatch
        .transition(CANCEL_BY_SYSTEM)
        .by_system()
        .reason(reason)
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dispatch_is_a_subject_with_a_number() {
        let dispatch = Dispatch {
            id: 7,
            source_type: "sales.sales_order".into(),
            source_id: 3,
            ..Dispatch::default()
        };
        assert_eq!(dispatch.subject(), Subject::new("inventory.dispatch", 7));
        assert_eq!(dispatch.number(), "DSP-0007");
        assert_eq!(dispatch.source(), Subject::new("sales.sales_order", 3));
    }

    #[test]
    fn only_draft_and_picked_goods_are_still_in_the_store() {
        assert!(DispatchStatus::Draft.in_store());
        assert!(DispatchStatus::Picked.in_store());
        assert!(!DispatchStatus::Dispatched.in_store());
        assert!(!DispatchStatus::Cancelled.in_store());
    }
}
