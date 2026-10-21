//! The Pending issue (wl-inventory §3 *Pending issue*, *Negative stock*,
//! issue #237): consumption another module posts automatically (a clinic
//! visit, an F&B sale, a work order material issue) must not fail because the
//! count is behind. What stock cannot cover becomes a pending issue for the
//! short quantity ([`crate::stock_service::consume`]), recorded with its
//! source and cost object.
//!
//! ```text
//! [*] --record--> open --post--> posted
//!                   |--clear / clear_approved / source_cancelled--> cleared
//! ```
//!
//! - `post` (system): stock arrived at the location (a goods receipt, a
//!   transfer; [`post_on_arrival`]); oldest first, as far as the arrival
//!   covers, a partly covered one split into the posted part and the open
//!   remainder. The `consume` movement posts at the average cost at posting
//!   time, from the first-expiry released lot for a lot-tracked item, which
//!   is recorded as *assigned on arrival*.
//! - `clear` (a holder of `stock.adjust` in the location's scope, with a
//!   reason): the stock was used but never on the books. An `adjust`
//!   movement adds the quantity at the current average cost (the lot named,
//!   for a lot-tracked item), then the pending consumption posts against it
//!   to its source and cost object, so the balance does not change. A value
//!   above the `STK.ACL.02` limit (default AED 1,000) turns `clear` into an
//!   approval request for a holder of `stock.approve` who is not the
//!   requester; nothing posts and the pending issue stays `open` until
//!   `clear_approved`.
//! - `source_cancelled` (system): the visit or sale was voided before the
//!   pending issue posted; it is `cleared` with the reason *source
//!   cancelled* and no movement ([`cancel_for_source`]).
//!
//! Decisions: a pending issue only exists where negative stock is blocked
//! (`allow_consumption` comes with BL-STK-19); a posted chunk of a split
//! pending issue is a row of its own pointing at the original
//! (`split_from_id`); a stock count's approved adjustments will call
//! [`post_on_arrival`] with `count` when the count lines are built.

use std::collections::BTreeSet;

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::PENDING_ISSUE_SUBJECT_TYPE;

use crate::balance;
use crate::item::{Item, value_of};
use crate::location::{Location, scope_of};
use crate::lot::Lot;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::texts::said;

/// The system's `post` transition's name.
pub const POST: &str = "post";

/// The user's `clear` transition's name.
pub const CLEAR: &str = "clear";

/// The system's `clear_approved` transition's name: the kernel runs it when
/// the approval of a clear above the value is given.
pub const CLEAR_APPROVED: &str = "clear_approved";

/// The system's `source_cancelled` transition's name.
pub const SOURCE_CANCELLED: &str = "source_cancelled";

/// The approval policy's action key of `clear` (`STK.ACL.02`).
pub const ACTION: &str = "inventory.pending_issue_clear";

/// The reason a pending issue is cleared with when its source is cancelled.
pub const SOURCE_CANCELLED_REASON: &str = "source cancelled";

/// Where a pending issue stands (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PendingStatus {
    /// Waiting for stock to arrive or for the storekeeper.
    #[default]
    Open,
    /// Posted against stock that arrived (terminal).
    Posted,
    /// Cleared by the storekeeper, or its source was cancelled (terminal).
    Cleared,
}

/// Consumption that stock could not cover.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_pending_issues",
    state_machine,
    index(status),
    index(location_id, item_id),
    index(source_type, source_id)
)]
pub struct PendingIssue {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The location the consumption was short at.
    pub location_id: i64,
    /// The type of the location's scope.
    pub scope_type: Option<String>,
    /// The id of the location's scope.
    pub scope_id: Option<i64>,
    /// The short quantity, in thousandths of the base unit.
    pub quantity_milli: i64,
    /// The type of the record that consumed.
    pub source_type: String,
    /// The id of the record that consumed.
    pub source_id: i64,
    /// The type of the cost object charged.
    pub cost_object_type: Option<String>,
    /// The id of the cost object charged.
    pub cost_object_id: Option<i64>,
    /// When it was recorded.
    pub recorded_at: DateTime,
    /// The lifecycle state; changed only by a transition.
    pub status: PendingStatus,
    /// The lot that covered it.
    pub lot_id: Option<i64>,
    /// The lot was chosen when stock arrived.
    pub assigned_on_arrival: bool,
    /// The pending issue a partly covered one was split from.
    pub split_from_id: Option<i64>,
    /// `receipt`, `transfer`, `count`, `storekeeper` or `source_cancelled`.
    pub resolved_by: Option<String>,
    /// The reason the storekeeper cleared it with.
    pub clear_reason: Option<String>,
    /// The user who cleared it.
    pub cleared_by: Option<i64>,
    /// When it posted or was cleared.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PendingIssue {
    /// The pending issue as a [`Subject`]: `inventory.pending_issue:5`.
    pub fn subject(&self) -> Subject {
        Subject::new(PENDING_ISSUE_SUBJECT_TYPE, self.id)
    }

    /// The record that consumed.
    pub fn source(&self) -> Subject {
        Subject::new(&self.source_type, self.source_id)
    }

    /// The record the cost is charged to.
    pub fn cost_object(&self) -> Option<Subject> {
        match (&self.cost_object_type, self.cost_object_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// Who may clear it: the scope of the location.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }
}

/// What a `post` run carries to its effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostInput {
    /// The lot that covers it, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// When it posts.
    pub at: DateTime,
    /// What stock arrived: `receipt`, `transfer` or `count`.
    pub via: String,
}

/// What a `clear` run carries to its guard, effect and approval policy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClearInput {
    /// The value of the quantity at the average cost, in fils: what the
    /// approval policy's condition reads (`STK.ACL.02`).
    pub amount: i64,
    /// The user who clears it (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The lot the stock is entered in, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// What is wrong with the lot, `None` when nothing is.
    pub lot_problem: Option<String>,
    /// Why the stock is entered.
    pub reason: String,
    /// When it is cleared.
    pub at: Option<DateTime>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

fn clear_input(ctx: &TransitionCtx) -> Option<ClearInput> {
    ctx.input_as::<ClearInput>().ok()
}

/// Adds the consume movement of `pending` for `lot_id` at `at`, posting at
/// the average cost of the item now.
async fn consume_in(
    tx: &mut Transaction,
    pending: &PendingIssue,
    item: &Item,
    lot_id: Option<i64>,
    at: DateTime,
    by: Option<i64>,
) -> Result {
    let mut moved = StockMovement::leaving(
        MovementType::Consume,
        item.id,
        lot_id,
        pending.location_id,
        pending.quantity_milli,
        item.average_cost,
        value_of(pending.quantity_milli, item.average_cost),
        &pending.source(),
        None,
        by,
    );
    moved.moved_at = at;
    moved.cost_object_type = pending.cost_object_type.clone();
    moved.cost_object_id = pending.cost_object_id;
    moved.insert(&mut *tx).await?;
    Ok(())
}

/// The effect of `post`: the pending consumption posts against the stock
/// that arrived.
async fn post_in(tx: &mut Transaction, pending: PendingIssue, run: &PostInput) -> Result {
    let item = Item::find_or_404(&mut *tx, pending.item_id).await?;
    let found =
        balance::balance(&mut *tx, item.id, pending.location_id, run.lot_id).await?;
    if found.available_milli() < pending.quantity_milli {
        return Err(refused(said("wl_inventory.pending.not_covered", &[])));
    }
    consume_in(tx, &pending, &item, run.lot_id, run.at, None).await?;
    let now = renox::db::now();
    PendingIssue::query()
        .where_(PendingIssue::ID.eq(pending.id))
        .update(
            &mut *tx,
            &[
                ("lot_id", &run.lot_id),
                ("assigned_on_arrival", &run.lot_id.is_some()),
                ("resolved_by", &Some(run.via.clone())),
                ("closed_at", &Some(run.at)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The effect of `clear` and `clear_approved`: the stock that was used but
/// never on the books is entered with an `adjust` at the average cost, and
/// the consumption posts against it.
async fn clear_in(tx: &mut Transaction, pending: PendingIssue, run: &ClearInput) -> Result {
    let item = Item::find_or_404(&mut *tx, pending.item_id).await?;
    let at = run.at.unwrap_or_else(renox::db::now);
    let mut entered = StockMovement::moving(
        MovementType::Adjust,
        item.id,
        run.lot_id,
        None,
        Some(pending.location_id),
        pending.quantity_milli,
        item.average_cost,
        value_of(pending.quantity_milli, item.average_cost),
        &pending.subject(),
        run.by,
    );
    entered.moved_at = at;
    entered.reason = Some(run.reason.clone());
    entered.insert(&mut *tx).await?;
    consume_in(tx, &pending, &item, run.lot_id, at, run.by).await?;
    let now = renox::db::now();
    PendingIssue::query()
        .where_(PendingIssue::ID.eq(pending.id))
        .update(
            &mut *tx,
            &[
                ("lot_id", &run.lot_id),
                ("resolved_by", &Some("storekeeper".to_owned())),
                ("clear_reason", &Some(run.reason.clone())),
                ("cleared_by", &run.by),
                ("closed_at", &Some(at)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// The shared rules of `clear` and `clear_approved`.
fn clear_rules(transition: Transition<PendingIssue>) -> Transition<PendingIssue> {
    transition
        .from([PendingStatus::Open])
        .to(PendingStatus::Cleared)
        .guard("lot_for_a_tracked_item", |_p: &PendingIssue, ctx| {
            let problem = match clear_input(ctx) {
                None => Some(said("wl_inventory.pending.not_open", &[])),
                Some(input) => input.lot_problem,
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|pending: &PendingIssue, ctx, tx| {
            let pending = pending.clone();
            let run = clear_input(ctx);
            Box::pin(async move {
                let run = run.ok_or_else(|| refused(said("wl_inventory.pending.not_open", &[])))?;
                clear_in(tx, pending, &run).await
            })
        })
}

impl StateMachine for PendingIssue {
    type State = PendingStatus;
    const COLUMN: renox::db::Col<PendingIssue, PendingStatus> = PendingIssue::STATUS;

    fn machine() -> Machine<Self> {
        use PendingStatus::{Cleared, Open, Posted};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(Posted)
            .tone("success")
            .terminal()
            .state(Cleared)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(POST)
                    .label("Post")
                    .system()
                    .from([Open])
                    .to(Posted)
                    .effect(|pending: &PendingIssue, ctx, tx| {
                        let pending = pending.clone();
                        let run = ctx.input_as::<PostInput>().ok();
                        Box::pin(async move {
                            let run = run
                                .ok_or_else(|| refused(said("wl_inventory.pending.not_open", &[])))?;
                            post_in(tx, pending, &run).await
                        })
                    }),
            )
            .transition(clear_rules(
                Transition::new(CLEAR)
                    .label("Clear")
                    .tone("primary")
                    .permission_in(permissions::STOCK_ADJUST, |p: &PendingIssue| p.scope())
                    .reason_required(),
            ))
            .transition(clear_rules(
                Transition::new(CLEAR_APPROVED)
                    .label("Clear (approved)")
                    .tone("primary")
                    .system(),
            ))
            .transition(
                Transition::new(SOURCE_CANCELLED)
                    .label("Source cancelled")
                    .system()
                    .from([Open])
                    .to(Cleared)
                    .effect(|pending: &PendingIssue, _ctx, tx| {
                        let id = pending.id;
                        Box::pin(async move {
                            let now = renox::db::now();
                            PendingIssue::query()
                                .where_(PendingIssue::ID.eq(id))
                                .update(
                                    &mut *tx,
                                    &[
                                        ("resolved_by", &Some("source_cancelled".to_owned())),
                                        (
                                            "clear_reason",
                                            &Some(SOURCE_CANCELLED_REASON.to_owned()),
                                        ),
                                        ("closed_at", &Some(now)),
                                        ("updated_at", &now),
                                    ],
                                )
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

/// Records `short_milli` of `item` at `location` as an `open` pending issue
/// for `source`, in `tx`.
///
/// # Errors
///
/// A database error.
pub(crate) async fn record_in(
    tx: &mut Transaction,
    location: &Location,
    item_id: i64,
    short_milli: i64,
    source: &Subject,
    cost_object: Option<&Subject>,
    at: DateTime,
) -> Result<PendingIssue> {
    let mut pending = PendingIssue {
        item_id,
        location_id: location.id,
        scope_type: location.scope_type.clone(),
        scope_id: location.scope_id,
        quantity_milli: short_milli,
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        cost_object_type: cost_object.map(|c| c.subject_type.clone()),
        cost_object_id: cost_object.map(|c| c.subject_id),
        recorded_at: at,
        status: PendingStatus::Open,
        ..PendingIssue::default()
    };
    pending.insert(&mut *tx).await?;
    Ok(pending)
}

/// The open pending issues, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn open_of(db: &Db) -> Result<Vec<PendingIssue>> {
    PendingIssue::query()
        .where_(PendingIssue::STATUS.eq(PendingStatus::Open))
        .order_by(PendingIssue::RECORDED_AT)
        .order_by(PendingIssue::ID)
        .get(db)
        .await
}

/// Posts the open pending issues of `item_ids` at `location_id`, oldest
/// first, as far as the stock that arrived covers them (`via`: `receipt`,
/// `transfer` or `count`). A pending issue the arrival covers in part is
/// split: the covered part posts, the rest stays open. Held quantity is never
/// used, and a lot-tracked item posts from the first-expiry released lot.
///
/// # Errors
///
/// A database error.
pub async fn post_on_arrival(
    state: &AppState,
    location_id: i64,
    item_ids: &[i64],
    via: &str,
) -> Result {
    let ids: BTreeSet<i64> = item_ids.iter().copied().collect();
    let at = renox::db::now();
    let mut tx = state.db.begin().await?;
    let mut outcomes = Vec::new();
    for item_id in ids {
        let item = Item::find_or_404(&mut tx, item_id).await?;
        let waiting = PendingIssue::query()
            .where_(PendingIssue::LOCATION_ID.eq(location_id))
            .where_(PendingIssue::ITEM_ID.eq(item_id))
            .where_(PendingIssue::STATUS.eq(PendingStatus::Open))
            .order_by(PendingIssue::RECORDED_AT)
            .order_by(PendingIssue::ID)
            .get(&mut tx)
            .await?;
        for pending in waiting {
            let mut remaining = pending.quantity_milli;
            while remaining > 0 {
                let candidates = balance::candidates(&mut tx, &item, location_id).await?;
                let Some(next) = candidates.first().copied() else {
                    break;
                };
                let chunk = remaining.min(next.available_milli);
                let target = if chunk == remaining {
                    PendingIssue::find_or_404(&mut tx, pending.id).await?
                } else {
                    // Partly covered: the covered part is a row of its own
                    // that posts, the rest stays open on the original.
                    let mut part = PendingIssue {
                        quantity_milli: chunk,
                        split_from_id: Some(pending.id),
                        status: PendingStatus::Open,
                        ..pending.clone()
                    };
                    part.id = 0;
                    part.insert(&mut tx).await?;
                    PendingIssue::query()
                        .where_(PendingIssue::ID.eq(pending.id))
                        .update(
                            &mut tx,
                            &[
                                ("quantity_milli", &(remaining - chunk)),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    part
                };
                let outcome = target
                    .transition(POST)
                    .by_system()
                    .input(&PostInput {
                        lot_id: next.lot_id,
                        at,
                        via: via.to_owned(),
                    })
                    .run_in(&mut tx)
                    .await
                    .map_err(TransitionError::into_error)?;
                outcomes.push(outcome);
                remaining -= chunk;
            }
        }
    }
    tx.commit().await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    Ok(())
}

/// Clears, with the reason *source cancelled* and no movement, the open
/// pending issues `source` left. Returns how many.
///
/// # Errors
///
/// A database error.
pub async fn cancel_for_source(state: &AppState, source: &Subject) -> Result<usize> {
    let open = PendingIssue::query()
        .where_(PendingIssue::SOURCE_TYPE.eq(source.subject_type.as_str()))
        .where_(PendingIssue::SOURCE_ID.eq(source.subject_id))
        .where_(PendingIssue::STATUS.eq(PendingStatus::Open))
        .order_by(PendingIssue::ID)
        .get(&state.db)
        .await?;
    let count = open.len();
    for pending in open {
        let _outcome = pending
            .transition(SOURCE_CANCELLED)
            .by_system()
            .reason(SOURCE_CANCELLED_REASON)
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(count)
}

/// What [`clear`] did.
#[derive(Debug, Clone)]
pub enum ClearOutcome {
    /// The pending issue is `cleared`: the stock was entered and the
    /// consumption posted.
    Cleared(PendingIssue),
    /// The value is above the limit: it stays `open` and waits for a second
    /// person's approval (`STK.ACL.02`).
    Waiting(ApprovalRequest),
}

/// `user` clears pending issue `id` with `reason`. For a lot-tracked item
/// `lot_id` names the lot the stock is entered in. Above the value it
/// becomes an approval request instead (see the module docs).
///
/// # Errors
///
/// `Forbidden` without `stock.adjust` in the location's scope; `NotFrom`
/// unless `open`; `ReasonRequired` without a reason; `Guard` (422) when a
/// lot-tracked item has no lot of its own named.
pub async fn clear(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
    lot_id: Option<i64>,
) -> std::result::Result<ClearOutcome, TransitionError> {
    let pending = PendingIssue::find_or_404(&state.db, id).await?;
    let item = Item::find_or_404(&state.db, pending.item_id).await?;
    let lot_problem = if item.lot_tracked {
        match lot_id {
            None => Some(said("wl_inventory.pending.lot_needed", &[])),
            Some(lot_id) => {
                let lot = Lot::find(&state.db, lot_id).await?;
                lot.is_none_or(|lot| lot.item_id != item.id)
                    .then(|| said("wl_inventory.pending.wrong_lot", &[]))
            }
        }
    } else {
        None
    };
    let run = ClearInput {
        amount: value_of(pending.quantity_milli, item.average_cost),
        by: Some(user.id),
        lot_id,
        lot_problem,
        reason: reason.trim().to_owned(),
        at: None,
    };
    let outcome = pending
        .transition(CLEAR)
        .by(user)
        .reason(reason.trim().to_owned())
        .input(&run)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => Ok(ClearOutcome::Cleared(
            PendingIssue::find_or_404(&state.db, id).await?,
        )),
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &pending.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(ClearOutcome::Waiting(request))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_issue_posts_or_is_cleared_and_only_the_system_posts() {
        let machine = PendingIssue::machine();
        assert_eq!(machine.initial_states(), ["open"]);
        let post = machine.find(POST).expect("post");
        assert!(post.is_system());
        assert_eq!(post.to_state(), Some("posted"));
        let clear = machine.find(CLEAR).expect("clear");
        assert!(!clear.is_system());
        assert_eq!(clear.to_state(), Some("cleared"));
        assert_eq!(clear.guard_names(), ["lot_for_a_tracked_item"]);
        for name in [CLEAR_APPROVED, SOURCE_CANCELLED] {
            assert!(machine.find(name).expect(name).is_system(), "{name}");
        }
        for state in ["posted", "cleared"] {
            let terminal = machine.states().iter().find(|s| s.name == state);
            assert!(terminal.is_some_and(|s| s.terminal), "{state}");
        }
    }
}
