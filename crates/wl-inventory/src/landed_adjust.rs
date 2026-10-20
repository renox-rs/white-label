//! Adjusting an allocated landed cost (wl-inventory §3 *Landed cost
//! adjustment*, §4 *Landed cost* `adjust`, §5.8, `STK.ACL.06`, issue #228). A
//! freight, duty or clearing bill often arrives after the landed cost was
//! allocated, or a duty refund comes later. The purchaser (`landed_costs.post`)
//! makes a [`LandedCostAdjustment`] with its cost lines (positive or
//! negative), the supplier bill's reference and a posting date, and posts it
//! with `adjust`:
//!
//! - the guards: the landed cost is `allocated`; the adjustment is its own
//!   and still a draft, with cost lines that do not add up to nothing and the
//!   supplier bill's reference; the posting date is in an open period
//!   (not in a VAT period filed in wl-invoicing, [`PeriodLocks`]);
//! - above the value (default AED 5,000, `STK.ACL.06`, the absolute amount, so
//!   a refund of AED 6,000 counts) `adjust` becomes an approval request for a
//!   second holder of `landed_costs.post`, never the requester; nothing posts
//!   until the approval, and a rejection leaves the adjustment a draft that
//!   can be posted again;
//! - the effect, in the transaction: the total is spread on the original
//!   basis (the weights kept at `allocate`, [`crate::landed_cost::spread`]),
//!   and per item the share on the quantity still on hand re-values the stock
//!   (the item's average cost), the share on the quantity already consumed is
//!   the landed cost variance, or goes to the cost object charged on receipt.
//!   A refund re-values stock down to zero value at most, the rest being
//!   variance. Earlier movements and the allocation are never changed; the
//!   adjustment is `posted`;
//! - after the commit ([`announce`]): the variance and the cost object are
//!   charged through the `CostPoster`, then `LandedCostAdjusted` follows.
//!
//! Decisions: the period is checked when `adjust` is asked, not again at the
//! approval (the final check has no access to the module providing the
//! locks); the final check only fails the request when the adjustment was
//! posted meanwhile. The adjustments of one landed cost are independent:
//! each is posted, approved and traced on its own.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::inventory::{
    GOODS_RECEIPT_SUBJECT_TYPE, ITEM_SUBJECT_TYPE, LANDED_COST_ADJUSTMENT_SUBJECT_TYPE,
    LOCATION_SUBJECT_TYPE, LandedCostAdjusted, LandedCostAllocated, LandedCostShare,
};
use wl_kernel::invoicing::{PostCost, period_locks, post_cost};

use crate::landed_cost::{
    CostLine, LandedCost, LandedCostReceipt, LandedCostStatus, NewCostLine, Share, allocations_in,
    apply_shares, insert_lines, parts, split, spread,
};
use crate::movement;
use crate::permissions;
use crate::texts::said;

/// The user's `adjust` transition's name.
pub const ADJUST: &str = "adjust";

/// The system's `adjust_approved` transition's name: the kernel runs it when
/// the approval of an adjustment above the value is given.
pub const ADJUST_APPROVED: &str = "adjust_approved";

/// The `post` transition's name of an adjustment.
pub const POST: &str = "post";

/// The approval policy's action key of `adjust` (`STK.ACL.06`).
pub const ACTION: &str = "inventory.landed_cost_adjust";

/// The cost code of the variance charged to the landed cost itself.
pub const VARIANCE_CODE: &str = "landed_cost_variance";

/// The cost code of the part charged to the cost object of the receipt.
pub const COST_OBJECT_CODE: &str = "landed_cost";

/// Where an adjustment stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentStatus {
    /// Being prepared, or waiting for approval.
    #[default]
    Draft,
    /// Spread over the items; stock re-valued (terminal).
    Posted,
}

/// A late or corrected import cost after allocation.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_landed_cost_adjustments",
    state_machine,
    index(landed_cost_id)
)]
pub struct LandedCostAdjustment {
    /// The primary key.
    pub id: i64,
    /// The landed cost.
    pub landed_cost_id: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: AdjustmentStatus,
    /// The supplier bill in wl-invoicing that caused it, if any.
    pub bill_id: Option<i64>,
    /// The supplier bill's reference.
    pub bill_reference: String,
    /// The date it is posted at.
    pub posting_date: NaiveDate,
    /// The sum of its cost lines in fils; negative for a refund.
    pub total: i64,
    /// Whether `LandedCostAdjusted` was sent and the variance posted.
    pub announced: bool,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// The user who posted it (who asked, when it waited for approval).
    pub posted_by: Option<i64>,
    /// When it was posted.
    pub posted_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LandedCostAdjustment {
    /// The adjustment as a [`Subject`]:
    /// `inventory.landed_cost_adjustment:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(LANDED_COST_ADJUSTMENT_SUBJECT_TYPE, self.id)
    }
}

impl StateMachine for LandedCostAdjustment {
    type State = AdjustmentStatus;
    const COLUMN: renox::db::Col<LandedCostAdjustment, AdjustmentStatus> =
        LandedCostAdjustment::STATUS;

    fn machine() -> Machine<Self> {
        use AdjustmentStatus::{Draft, Posted};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Posted)
            .tone("success")
            .terminal()
            .transition(
                Transition::new(POST)
                    .label("Post")
                    .system()
                    .from([Draft])
                    .to(Posted),
            )
    }
}

/// What the user enters to make an adjustment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewAdjustment {
    /// The supplier bill's reference.
    pub bill_reference: String,
    /// The supplier bill in wl-invoicing, if it came from one.
    pub bill_id: Option<i64>,
    /// The date it is posted at.
    pub posting_date: NaiveDate,
    /// The cost lines, positive or negative.
    pub lines: Vec<NewCostLine>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Makes a draft adjustment of landed cost `landed_cost_id`.
///
/// # Errors
///
/// 422 when the landed cost is not `allocated` or `new` has no cost line; a
/// database error.
pub async fn create_adjustment(
    db: &Db,
    by: Option<i64>,
    landed_cost_id: i64,
    new: NewAdjustment,
) -> Result<LandedCostAdjustment> {
    let landed = LandedCost::find_or_404(db, landed_cost_id).await?;
    if landed.status != LandedCostStatus::Allocated {
        return Err(refused(said("wl_inventory.landed.not_allocated", &[])));
    }
    if new.lines.is_empty() {
        return Err(refused(said("wl_inventory.landed.no_lines", &[])));
    }
    let mut tx = db.begin().await?;
    let mut adjustment = LandedCostAdjustment {
        landed_cost_id,
        bill_id: new.bill_id,
        bill_reference: new.bill_reference.trim().to_owned(),
        posting_date: new.posting_date,
        total: new.lines.iter().map(|l| l.amount).sum(),
        created_by: by,
        ..LandedCostAdjustment::default()
    };
    adjustment.insert(&mut tx).await?;
    insert_lines(&mut tx, landed_cost_id, Some(adjustment.id), &new.lines).await?;
    tx.commit().await?;
    Ok(adjustment)
}

/// The cost lines of adjustment `adjustment_id`, in order.
///
/// # Errors
///
/// A database error.
pub async fn lines_of(db: &Db, adjustment_id: i64) -> Result<Vec<CostLine>> {
    CostLine::query()
        .where_(CostLine::ADJUSTMENT_ID.eq(adjustment_id))
        .order_by(CostLine::POSITION)
        .get(db)
        .await
}

/// The shares an adjustment (or, with `None`, the allocation) put on the
/// items, in order.
///
/// # Errors
///
/// A database error.
pub async fn shares_of(
    db: &Db,
    landed_cost_id: i64,
    adjustment_id: Option<i64>,
) -> Result<Vec<Share>> {
    let query = Share::query().where_(Share::LANDED_COST_ID.eq(landed_cost_id));
    let query = match adjustment_id {
        Some(id) => query.where_(Share::ADJUSTMENT_ID.eq(id)),
        None => query.where_(Share::ADJUSTMENT_ID.is_null()),
    };
    query.order_by(Share::ID).get(db).await
}

/// What `adjust` would do to one item at one location now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The item.
    pub item_id: i64,
    /// Where it was received.
    pub location_id: i64,
    /// The whole share.
    pub amount: i64,
    /// The part on the quantity still on hand (before the zero-value stop of
    /// a refund).
    pub on_hand: i64,
    /// The part on the quantity already consumed.
    pub consumed: i64,
}

/// The split of adjustment `adjustment` over the items, as it is now, so that
/// the purchaser sees what stays on stock and what is consumed before
/// posting.
///
/// # Errors
///
/// A database error.
pub async fn preview(
    tx: &mut Transaction,
    landed_cost_id: i64,
    total: i64,
) -> Result<Vec<Preview>> {
    let rows = allocations_in(tx, landed_cost_id).await?;
    let weights: Vec<i64> = rows.iter().map(|row| row.weight).collect();
    let amounts = spread(total, &weights);
    let mut found = Vec::new();
    for part in parts(&rows, &amounts) {
        let here = movement::on_hand_in(tx, part.item_id, part.location_id).await?;
        let (on_hand, consumed) = split(part.amount, part.received_milli, here);
        found.push(Preview {
            item_id: part.item_id,
            location_id: part.location_id,
            amount: part.amount,
            on_hand,
            consumed,
        });
    }
    Ok(found)
}

/// What an `adjust` run carries to its guards and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AdjustInput {
    /// The adjustment to post.
    pub adjustment_id: i64,
    /// The user who posts it (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The sum of its cost lines in fils.
    pub total: i64,
    /// The amount the approval policy's condition reads: the absolute value
    /// of the total (`STK.ACL.06`).
    pub amount: i64,
    /// The first thing wrong, `None` when nothing is.
    pub problem: Option<String>,
}

/// What stands in the way of posting adjustment `adjustment_id` of `landed`,
/// read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    landed: &LandedCost,
    adjustment_id: i64,
    by: Option<i64>,
) -> Result<AdjustInput> {
    let mut input = AdjustInput {
        adjustment_id,
        by,
        ..AdjustInput::default()
    };
    let Some(adjustment) = LandedCostAdjustment::find(&mut *tx, adjustment_id).await? else {
        input.problem = Some(said("wl_inventory.landed.no_adjustment", &[]));
        return Ok(input);
    };
    let lines = CostLine::query()
        .where_(CostLine::ADJUSTMENT_ID.eq(adjustment.id))
        .order_by(CostLine::POSITION)
        .get(&mut *tx)
        .await?;
    input.total = lines.iter().map(|l| l.amount).sum();
    input.amount = input.total.abs();
    input.problem = if landed.status != LandedCostStatus::Allocated {
        Some(said("wl_inventory.landed.not_allocated", &[]))
    } else if adjustment.landed_cost_id != landed.id {
        Some(said("wl_inventory.landed.no_adjustment", &[]))
    } else if adjustment.status != AdjustmentStatus::Draft {
        Some(said("wl_inventory.landed.not_draft", &[]))
    } else if lines.is_empty() {
        Some(said("wl_inventory.landed.no_lines", &[]))
    } else if adjustment.bill_reference.trim().is_empty() {
        Some(said("wl_inventory.landed.bill_reference", &[]))
    } else if input.total == 0 {
        Some(said("wl_inventory.landed.zero_total", &[]))
    } else {
        None
    };
    Ok(input)
}

fn input(ctx: &TransitionCtx) -> Option<AdjustInput> {
    ctx.input_as::<AdjustInput>().ok()
}

/// The guards and the effect `adjust` and `adjust_approved` share.
fn rules(transition: Transition<LandedCost>) -> Transition<LandedCost> {
    transition
        .from([LandedCostStatus::Allocated])
        .to(LandedCostStatus::Allocated)
        .guard("adjustment_allowed", |_landed: &LandedCost, ctx| {
            let problem = input(ctx).map_or_else(
                || Some(said("wl_inventory.landed.bad_input", &[])),
                |input| input.problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|landed: &LandedCost, ctx, tx| {
            let landed = landed.clone();
            let run = input(ctx).unwrap_or_default();
            Box::pin(async move { adjust_in(tx, landed, &run).await })
        })
}

/// Adds `adjust` and its approved twin to the landed cost's machine.
pub(crate) fn add_to(machine: Machine<LandedCost>) -> Machine<LandedCost> {
    machine
        .transition(rules(
            Transition::new(ADJUST)
                .label("Adjust")
                .tone("primary")
                .permission(permissions::LANDED_COSTS_POST),
        ))
        .transition(rules(
            Transition::new(ADJUST_APPROVED)
                .label("Adjust (approved)")
                .system(),
        ))
}

/// The effect of `adjust` and `adjust_approved`, in the run's transaction.
async fn adjust_in(tx: &mut Transaction, landed: LandedCost, run: &AdjustInput) -> Result {
    // The adjustment may have been posted since the guards looked.
    let found = gather(tx, &landed, run.adjustment_id, run.by).await?;
    if let Some(problem) = found.problem {
        return Err(refused(problem));
    }
    let now = renox::db::now();
    let rows = allocations_in(tx, landed.id).await?;
    let weights: Vec<i64> = rows.iter().map(|row| row.weight).collect();
    let amounts = spread(found.total, &weights);
    apply_shares(tx, landed.id, Some(run.adjustment_id), &rows, &amounts).await?;
    LandedCostAdjustment::query()
        .where_(LandedCostAdjustment::ID.eq(run.adjustment_id))
        .update(
            &mut *tx,
            &[
                ("total", &found.total),
                ("posted_by", &run.by),
                ("posted_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    let adjustment = LandedCostAdjustment::find_or_404(&mut *tx, run.adjustment_id).await?;
    let _outcome = adjustment
        .transition(POST)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// What [`adjust`] did.
#[derive(Debug, Clone)]
pub enum AdjustOutcome {
    /// The adjustment is `posted`: stock re-valued, the variance kept.
    Posted(LandedCostAdjustment),
    /// The amount is above the value: nothing posts until a second person
    /// approves (`STK.ACL.06`).
    Waiting(ApprovalRequest),
}

/// The request waiting for adjustment `adjustment_id` of `landed`, if any.
async fn waiting_for(
    db: &Db,
    landed: &LandedCost,
    adjustment_id: i64,
) -> Result<Option<ApprovalRequest>> {
    Ok(ApprovalRequest::pending_about(db, &landed.subject())
        .await?
        .into_iter()
        .find(|request| {
            request.action == ACTION
                && request
                    .payload
                    .0
                    .get("adjustment_id")
                    .and_then(|v| v.as_i64())
                    == Some(adjustment_id)
        }))
}

/// `user` posts adjustment `adjustment_id` of landed cost `landed_cost_id`.
/// Refused with the first problem before anything is asked or written; above
/// the value it becomes an approval request instead.
///
/// # Errors
///
/// `Forbidden` without `landed_costs.post`; `Guard` (422) for a problem of
/// the adjustment or a posting date in a locked period.
pub async fn adjust(
    state: &AppState,
    user: &AuthUser,
    landed_cost_id: i64,
    adjustment_id: i64,
) -> std::result::Result<AdjustOutcome, TransitionError> {
    let landed = LandedCost::find_or_404(&state.db, landed_cost_id).await?;
    if let Some(adjustment) = LandedCostAdjustment::find(&state.db, adjustment_id).await?
        && period_locks(state)
            .is_locked(&state.db, landed.legal_entity_id, adjustment.posting_date)
            .await?
    {
        return Err(TransitionError::Guard(said(
            "wl_inventory.landed.period_locked",
            &[],
        )));
    }
    if waiting_for(&state.db, &landed, adjustment_id)
        .await?
        .is_some()
    {
        return Err(TransitionError::Guard(said(
            "wl_inventory.landed.pending",
            &[],
        )));
    }
    // Reads in a transaction of its own, rolled back: nothing is held while
    // the transition opens its own.
    let mut tx = state.db.begin().await?;
    let found = gather(&mut tx, &landed, adjustment_id, Some(user.id)).await?;
    tx.rollback().await?;
    if let Some(problem) = &found.problem {
        return Err(TransitionError::Guard(problem.clone()));
    }
    let outcome = landed
        .transition(ADJUST)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => {
            let done = LandedCostAdjustment::find_or_404(&state.db, adjustment_id).await?;
            Ok(AdjustOutcome::Posted(done))
        }
        Outcome::Pending(_) => {
            let request = waiting_for(&state.db, &landed, adjustment_id)
                .await?
                .ok_or(Error::NotFound)?;
            Ok(AdjustOutcome::Waiting(request))
        }
    }
}

/// The final check of a request to post an adjustment: it fails when the
/// adjustment was posted meanwhile.
///
/// # Errors
///
/// A database error.
pub async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    if request.action != ACTION {
        return Ok(None);
    }
    let asked: AdjustInput =
        renox::serde_json::from_value(request.payload.0.clone()).unwrap_or_default();
    let landed = LandedCost::find_or_404(&mut *tx, request.subject_id).await?;
    Ok(gather(tx, &landed, asked.adjustment_id, asked.by)
        .await?
        .problem)
}

/// `shares` as the kernel event gives them.
fn event_shares(shares: &[Share]) -> Vec<LandedCostShare> {
    shares
        .iter()
        .map(|share| LandedCostShare {
            item: Subject::new(ITEM_SUBJECT_TYPE, share.item_id),
            location: Subject::new(LOCATION_SUBJECT_TYPE, share.location_id),
            amount: share.amount,
            revalued: share.revalued,
            variance: share.variance,
        })
        .collect()
}

/// Charges the part of `shares` on goods already consumed: to the cost object
/// charged on receipt when the receipt had one, else to the landed cost
/// variance. One key per share, so a repeat posts nothing twice.
async fn post_variances(state: &AppState, landed: &LandedCost, shares: &[Share]) -> Result {
    for share in shares.iter().filter(|share| share.variance != 0) {
        let (cost_object, code) = match share.cost_object() {
            Some(cost_object) => (cost_object, COST_OBJECT_CODE),
            None => (landed.subject(), VARIANCE_CODE),
        };
        let request = PostCost::new(
            landed.subject(),
            format!("inventory.landed_cost_share:{}", share.id),
            cost_object,
            share.variance,
        )
        .cost_code(code);
        post_cost(state, request).await?;
    }
    Ok(())
}

/// After a commit: posts the variance and sends `LandedCostAllocated` for an
/// allocation not announced yet, and `LandedCostAdjusted` for every posted
/// adjustment not announced yet, read as committed. Repeating it does
/// nothing.
///
/// # Errors
///
/// A database error, or the cost poster's.
pub async fn announce(state: &AppState, landed_cost_id: i64) -> Result {
    let db = &state.db;
    let landed = LandedCost::find_or_404(db, landed_cost_id).await?;
    if landed.status == LandedCostStatus::Allocated && !landed.announced {
        let shares = shares_of(db, landed.id, None).await?;
        post_variances(state, &landed, &shares).await?;
        let receipts = LandedCostReceipt::query()
            .where_(LandedCostReceipt::LANDED_COST_ID.eq(landed.id))
            .order_by(LandedCostReceipt::ID)
            .get(db)
            .await?;
        LandedCost::query()
            .where_(LandedCost::ID.eq(landed.id))
            .update(db, &[("announced", &true)])
            .await?;
        state
            .emit(LandedCostAllocated {
                landed_cost: landed.subject(),
                receipts: receipts
                    .iter()
                    .map(|r| Subject::new(GOODS_RECEIPT_SUBJECT_TYPE, r.goods_receipt_id))
                    .collect(),
                amount: shares.iter().map(|s| s.amount).sum(),
                shares: event_shares(&shares),
            })
            .await?;
    }
    let posted = LandedCostAdjustment::query()
        .where_(LandedCostAdjustment::LANDED_COST_ID.eq(landed.id))
        .where_(LandedCostAdjustment::STATUS.eq(AdjustmentStatus::Posted))
        .where_(LandedCostAdjustment::ANNOUNCED.eq(false))
        .order_by(LandedCostAdjustment::ID)
        .get(db)
        .await?;
    for adjustment in posted {
        let shares = shares_of(db, landed.id, Some(adjustment.id)).await?;
        post_variances(state, &landed, &shares).await?;
        LandedCostAdjustment::query()
            .where_(LandedCostAdjustment::ID.eq(adjustment.id))
            .update(db, &[("announced", &true)])
            .await?;
        state
            .emit(LandedCostAdjusted {
                landed_cost: landed.subject(),
                adjustment: adjustment.subject(),
                amount: adjustment.total,
                shares: event_shares(&shares),
            })
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjust_is_a_user_transition_and_its_twin_a_system_one() {
        let machine = LandedCost::machine();
        let user = machine.find(ADJUST).expect("adjust");
        let twin = machine.find(ADJUST_APPROVED).expect("adjust_approved");
        assert!(!user.is_system());
        assert!(twin.is_system());
        for transition in [user, twin] {
            assert_eq!(transition.from_states(), ["allocated"]);
            assert_eq!(transition.to_state(), Some("allocated"));
            assert_eq!(transition.guard_names(), ["adjustment_allowed"]);
        }
    }

    #[test]
    fn an_adjustment_is_posted_by_the_system_only() {
        let machine = LandedCostAdjustment::machine();
        let post = machine.find(POST).expect("post");
        assert!(post.is_system());
        assert_eq!(post.from_states(), ["draft"]);
        assert_eq!(post.to_state(), Some("posted"));
        assert_eq!(machine.initial_states(), ["draft"]);
    }
}
