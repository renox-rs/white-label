//! The Landed cost (wl-inventory §3, §4 *Landed cost*): import costs
//! (freight, duty, clearing, insurance) spread over the items of received
//! goods, so that stock is valued at what it really cost. Only what the
//! adjustments of issues #228 and #229 stand on is built: a draft with its
//! receipts and cost lines, and `allocate`; the adjustments are in
//! [`crate::landed_adjust`].
//!
//! ```text
//! [*] --create--> draft --allocate--> allocated --adjust--> allocated
//!                 draft --cancel--> cancelled
//! ```
//!
//! `allocate` (`landed_costs.post`) spreads the total of the cost lines over
//! the accepted lines of the receipts on the chosen basis: `value` (accepted
//! quantity times unit cost), `quantity`, or `weight` / `volume` (given per
//! receipt line, as items carry neither). Rounding: each line's share is
//! rounded down, and the remainder of a few fils goes to the line with the
//! largest share, the first on a tie ([`spread`]). Per item and location the
//! share is then split ([`split`]): the part on the quantity still on hand
//! re-values the stock (the item's average cost, [`apply_shares`]), the part
//! on the quantity already consumed goes to the landed cost variance, or to
//! the cost object charged on receipt (the scope of a site store's receipt).
//! "Still on hand" is the smaller of what is on hand at the receipt's
//! location now and what the receipts brought, over what they brought:
//! average cost keeps no layers of receipts.
//!
//! Decisions: amounts are in AED, the currency of stock value; the
//! variance and the cost object are charged through the `CostPoster` after
//! the commit ([`crate::landed_adjust::announce`]); `LandedCostAllocated`
//! follows. A negative total works the same way, but the stock is re-valued
//! down to zero value at most and the rest goes to the variance.

use std::collections::BTreeMap;

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::LANDED_COST_SUBJECT_TYPE;
use wl_kernel::{LegalEntity, Subject};

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use crate::item::{Item, average_after_revalue, value_of};
use crate::movement;
use crate::permissions;
use crate::texts::said;

/// The `allocate` transition's name.
pub const ALLOCATE: &str = "allocate";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// Where a landed cost stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LandedCostStatus {
    /// Costs and receipts being collected.
    #[default]
    Draft,
    /// Costs spread over the received items; late costs can be adjusted.
    Allocated,
    /// Discarded (terminal).
    Cancelled,
}

/// How a landed cost is spread over the receipt lines (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AllocationBasis {
    /// By accepted quantity times unit cost.
    #[default]
    Value,
    /// By accepted quantity.
    Quantity,
    /// By weight, given per receipt line.
    Weight,
    /// By volume, given per receipt line.
    Volume,
}

impl AllocationBasis {
    /// Whether the weight of a line is given by the user.
    pub fn is_measured(self) -> bool {
        matches!(self, AllocationBasis::Weight | AllocationBasis::Volume)
    }
}

/// What a cost line is for.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CostKind {
    /// Freight.
    #[default]
    Freight,
    /// Customs duty.
    Duty,
    /// Clearing charges.
    Clearing,
    /// Insurance.
    Insurance,
}

/// Import costs spread over received goods.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_landed_costs",
    state_machine,
    unique(number),
    index(status)
)]
pub struct LandedCost {
    /// The primary key.
    pub id: i64,
    /// The number from the `LC-` series.
    pub number: Option<String>,
    /// The legal entity that owns the cost.
    pub legal_entity_id: i64,
    /// How the cost is spread.
    pub basis: AllocationBasis,
    /// The lifecycle state; changed only by a transition.
    pub status: LandedCostStatus,
    /// Whether the allocation was announced (event and cost postings).
    pub announced: bool,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// The user who allocated it.
    pub allocated_by: Option<i64>,
    /// When it was allocated.
    pub allocated_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LandedCost {
    /// The landed cost as a [`Subject`]: `inventory.landed_cost:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(LANDED_COST_SUBJECT_TYPE, self.id)
    }
}

/// A receipt a landed cost is spread over.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_landed_cost_receipts",
    unique(landed_cost_id, goods_receipt_id)
)]
pub struct LandedCostReceipt {
    /// The primary key.
    pub id: i64,
    /// The landed cost.
    pub landed_cost_id: i64,
    /// The goods receipt.
    pub goods_receipt_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One cost line: of the allocation (no adjustment) or of an adjustment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_landed_cost_lines", index(landed_cost_id))]
pub struct CostLine {
    /// The primary key.
    pub id: i64,
    /// The landed cost.
    pub landed_cost_id: i64,
    /// The adjustment, for a line of an adjustment.
    pub adjustment_id: Option<i64>,
    /// The order of the line, from 1.
    pub position: i64,
    /// What it is for.
    pub kind: CostKind,
    /// A description.
    pub description: String,
    /// The amount in fils; negative for a refund or a credit.
    pub amount: i64,
    /// The supplier bill line in wl-invoicing, if any.
    pub bill_line_id: Option<i64>,
    /// The supplier bill's reference.
    pub bill_reference: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What the user enters for one cost line.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NewCostLine {
    /// What it is for.
    pub kind: CostKind,
    /// A description.
    pub description: String,
    /// The amount in fils; negative for a refund or a credit.
    pub amount: i64,
    /// The supplier bill line, if any.
    pub bill_line_id: Option<i64>,
    /// The supplier bill's reference.
    pub bill_reference: Option<String>,
}

impl NewCostLine {
    /// A line of `kind` for `amount` fils.
    pub fn of(kind: CostKind, amount: i64) -> Self {
        NewCostLine {
            kind,
            amount,
            ..NewCostLine::default()
        }
    }

    /// With the supplier bill's reference.
    pub fn bill(mut self, reference: &str) -> Self {
        self.bill_reference = Some(reference.trim().to_owned());
        self
    }
}

/// How the cost was spread over one accepted receipt line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_landed_cost_allocations", index(landed_cost_id))]
pub struct Allocation {
    /// The primary key.
    pub id: i64,
    /// The landed cost.
    pub landed_cost_id: i64,
    /// The receipt line.
    pub receipt_line_id: i64,
    /// The item.
    pub item_id: i64,
    /// Where the goods were received.
    pub location_id: i64,
    /// The type of the cost object charged on receipt, copied from the
    /// receipt's scope.
    pub cost_object_type: Option<String>,
    /// The id of the cost object charged on receipt.
    pub cost_object_id: Option<i64>,
    /// What the line accepted, in thousandths.
    pub quantity_milli: i64,
    /// The weight of the line on the basis.
    pub weight: i64,
    /// What the allocation put on the line, in fils.
    pub amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What the allocation or an adjustment put on an item at a location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_landed_cost_shares",
    index(landed_cost_id, adjustment_id)
)]
pub struct Share {
    /// The primary key.
    pub id: i64,
    /// The landed cost.
    pub landed_cost_id: i64,
    /// The adjustment; `None` for the allocation.
    pub adjustment_id: Option<i64>,
    /// The item.
    pub item_id: i64,
    /// Where the goods were received.
    pub location_id: i64,
    /// The type of the cost object charged on receipt.
    pub cost_object_type: Option<String>,
    /// The id of the cost object charged on receipt.
    pub cost_object_id: Option<i64>,
    /// The whole share, in fils.
    pub amount: i64,
    /// The part that re-valued stock still on hand.
    pub revalued: i64,
    /// The part on goods already consumed.
    pub variance: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Share {
    /// The cost object charged on receipt, if the receipt's location is in a
    /// scope.
    pub fn cost_object(&self) -> Option<Subject> {
        match (&self.cost_object_type, self.cost_object_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }
}

/// `total` fils spread over `weights`: every share rounded down in
/// magnitude, the remainder to the largest share (the first on a tie). A
/// negative `total` gives negative shares; nothing is spread over no weight.
pub fn spread(total: i64, weights: &[i64]) -> Vec<i64> {
    let sum: i128 = weights.iter().map(|w| i128::from((*w).max(0))).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let magnitude = i128::from(total.unsigned_abs());
    let mut shares: Vec<i128> = weights
        .iter()
        .map(|w| magnitude * i128::from((*w).max(0)) / sum)
        .collect();
    let remainder = magnitude - shares.iter().sum::<i128>();
    let largest = (0..shares.len()).max_by(|a, b| shares[*a].cmp(&shares[*b]).then(b.cmp(a)));
    if let Some(index) = largest {
        shares[index] += remainder;
    }
    let sign: i128 = if total < 0 { -1 } else { 1 };
    shares
        .into_iter()
        .map(|share| i64::try_from(share * sign).unwrap_or(0))
        .collect()
}

/// `n` over `d`, rounded half away from zero.
fn divide_rounded(n: i128, d: i128) -> i64 {
    let magnitude = (2 * n.abs() + d) / (2 * d);
    i64::try_from(if n < 0 { -magnitude } else { magnitude }).unwrap_or(0)
}

/// Splits `share` of an item received in `received_milli` thousandths of
/// which `on_hand_milli` are still at the location: the part on the
/// quantity still on hand and the part on the quantity consumed. The consumed
/// part is rounded to fils and the on-hand part takes the remainder.
pub fn split(share: i64, received_milli: i64, on_hand_milli: i64) -> (i64, i64) {
    if received_milli <= 0 {
        return (0, share);
    }
    let kept = on_hand_milli.clamp(0, received_milli);
    let consumed = divide_rounded(
        i128::from(share) * i128::from(received_milli - kept),
        i128::from(received_milli),
    );
    (share - consumed, consumed)
}

/// What a share did, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The item.
    pub item_id: i64,
    /// Where it was received.
    pub location_id: i64,
    /// The cost object charged on receipt.
    pub cost_object_type: Option<String>,
    /// Its id.
    pub cost_object_id: Option<i64>,
    /// The whole share.
    pub amount: i64,
    /// Received on the allocated receipts, in thousandths.
    pub received_milli: i64,
}

/// The shares of `rows` for `amounts` (one per row), grouped per item and
/// location in the order the rows come.
pub fn parts(rows: &[Allocation], amounts: &[i64]) -> Vec<Part> {
    let mut found: Vec<Part> = Vec::new();
    for (row, amount) in rows.iter().zip(amounts) {
        match found
            .iter_mut()
            .find(|p| p.item_id == row.item_id && p.location_id == row.location_id)
        {
            Some(part) => {
                part.amount += amount;
                part.received_milli += row.quantity_milli;
            }
            None => found.push(Part {
                item_id: row.item_id,
                location_id: row.location_id,
                cost_object_type: row.cost_object_type.clone(),
                cost_object_id: row.cost_object_id,
                amount: *amount,
                received_milli: row.quantity_milli,
            }),
        }
    }
    found
}

/// Puts `amounts` (one per row of `rows`) on the stock and stores the
/// shares: per item and location, the part on the quantity still on hand
/// re-values the item (its average cost), the rest is the variance. Earlier
/// movements are not touched. `adjustment_id` is `None` for the allocation.
///
/// # Errors
///
/// A database error.
pub async fn apply_shares(
    tx: &mut Transaction,
    landed_cost_id: i64,
    adjustment_id: Option<i64>,
    rows: &[Allocation],
    amounts: &[i64],
) -> Result<Vec<Share>> {
    let now = renox::db::now();
    let mut stored = Vec::new();
    for part in parts(rows, amounts) {
        let on_hand_here = movement::on_hand_in(tx, part.item_id, part.location_id).await?;
        let (revalued, _) = split(part.amount, part.received_milli, on_hand_here);
        let item = Item::find_or_404(&mut *tx, part.item_id).await?;
        let on_hand_all = movement::on_hand_everywhere_in(tx, item.id).await?;
        // A refund re-values stock down to zero value at most.
        let applied = if on_hand_all <= 0 {
            0
        } else if revalued < 0 {
            revalued.max(-value_of(on_hand_all, item.average_cost))
        } else {
            revalued
        };
        if applied != 0 {
            let average = average_after_revalue(on_hand_all, item.average_cost, applied);
            Item::query()
                .where_(Item::ID.eq(item.id))
                .update(
                    &mut *tx,
                    &[("average_cost", &average), ("updated_at", &now)],
                )
                .await?;
        }
        let mut share = Share {
            landed_cost_id,
            adjustment_id,
            item_id: part.item_id,
            location_id: part.location_id,
            cost_object_type: part.cost_object_type.clone(),
            cost_object_id: part.cost_object_id,
            amount: part.amount,
            revalued: applied,
            variance: part.amount - applied,
            ..Share::default()
        };
        share.insert(&mut *tx).await?;
        stored.push(share);
    }
    Ok(stored)
}

/// The allocation rows of landed cost `landed_cost_id`, in order, read in
/// `tx`.
pub async fn allocations_in(tx: &mut Transaction, landed_cost_id: i64) -> Result<Vec<Allocation>> {
    Allocation::query()
        .where_(Allocation::LANDED_COST_ID.eq(landed_cost_id))
        .order_by(Allocation::ID)
        .get(&mut *tx)
        .await
}

/// The cost lines of the allocation of landed cost `landed_cost_id`.
pub async fn cost_lines_in(tx: &mut Transaction, landed_cost_id: i64) -> Result<Vec<CostLine>> {
    CostLine::query()
        .where_(CostLine::LANDED_COST_ID.eq(landed_cost_id))
        .where_(CostLine::ADJUSTMENT_ID.is_null())
        .order_by(CostLine::POSITION)
        .get(&mut *tx)
        .await
}

/// What the user enters to make a landed cost.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewLandedCost {
    /// How to spread it.
    pub basis: AllocationBasis,
    /// The goods receipts it is spread over.
    pub receipts: Vec<i64>,
    /// The cost lines.
    pub lines: Vec<NewCostLine>,
}

/// Makes a draft landed cost of `new`, numbered `LC-2026-0001`, owned by the
/// default legal entity.
///
/// # Errors
///
/// 422 when it names no receipt or no cost line; a database error.
pub async fn create_draft(db: &Db, by: Option<i64>, new: NewLandedCost) -> Result<LandedCost> {
    if new.receipts.is_empty() || new.lines.is_empty() {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            said("wl_inventory.landed.draft_needs", &[]),
        ));
    }
    let entity = LegalEntity::default_entity(db).await?;
    let mut tx = db.begin().await?;
    let number = LANDED_COST_SERIES
        .next(&mut tx, None, None, wl_kernel::documents::today())
        .await?;
    let mut landed = LandedCost {
        number: Some(number),
        legal_entity_id: entity.id,
        basis: new.basis,
        created_by: by,
        ..LandedCost::default()
    };
    landed.insert(&mut tx).await?;
    for receipt_id in &new.receipts {
        let mut link = LandedCostReceipt {
            landed_cost_id: landed.id,
            goods_receipt_id: *receipt_id,
            ..LandedCostReceipt::default()
        };
        link.insert(&mut tx).await?;
    }
    insert_lines(&mut tx, landed.id, None, &new.lines).await?;
    tx.commit().await?;
    Ok(landed)
}

/// Stores `lines` as cost lines of landed cost `landed_cost_id` (of the
/// adjustment, if one is given), numbered from 1.
///
/// # Errors
///
/// A database error.
pub async fn insert_lines(
    tx: &mut Transaction,
    landed_cost_id: i64,
    adjustment_id: Option<i64>,
    lines: &[NewCostLine],
) -> Result {
    for (position, line) in (1..).zip(lines) {
        let mut row = CostLine {
            landed_cost_id,
            adjustment_id,
            position,
            kind: line.kind,
            description: line.description.clone(),
            amount: line.amount,
            bill_line_id: line.bill_line_id,
            bill_reference: line.bill_reference.clone(),
            ..CostLine::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(())
}

/// The numbers of landed costs: `LC-2026-0001`.
pub const LANDED_COST_SERIES: wl_kernel::numbering::Series = wl_kernel::numbering::Series {
    key: "landed_cost",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "LC",
    reset: wl_kernel::numbering::Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// What an `allocate` run carries to its guards and effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AllocateInput {
    /// The weight or volume of receipt lines, `(receipt line, weight)`, for
    /// the `weight` and `volume` bases.
    pub weights: Vec<(i64, i64)>,
    /// The user who allocates.
    pub by: Option<i64>,
    /// The first thing wrong, `None` when nothing is.
    pub problem: Option<String>,
}

impl AllocateInput {
    /// An allocation with the given measured `weights`.
    pub fn weighted(weights: &[(i64, i64)]) -> Self {
        AllocateInput {
            weights: weights.to_vec(),
            ..AllocateInput::default()
        }
    }
}

/// The accepted lines of the receipts of landed cost `landed_cost_id` with
/// the receipt they are on, in receipt then line order, read in `tx`.
async fn accepted_lines(
    tx: &mut Transaction,
    landed_cost_id: i64,
) -> Result<(Vec<(GoodsReceipt, GoodsReceiptLine)>, Option<String>)> {
    let links = LandedCostReceipt::query()
        .where_(LandedCostReceipt::LANDED_COST_ID.eq(landed_cost_id))
        .order_by(LandedCostReceipt::ID)
        .get(&mut *tx)
        .await?;
    let mut found = Vec::new();
    for link in links {
        let receipt = GoodsReceipt::find_or_404(&mut *tx, link.goods_receipt_id).await?;
        if receipt.status != GoodsReceiptStatus::Posted {
            let number = receipt.number.clone().unwrap_or_default();
            return Ok((
                Vec::new(),
                Some(said(
                    "wl_inventory.landed.receipt_not_posted",
                    &[("receipt", &number)],
                )),
            ));
        }
        let lines = GoodsReceiptLine::query()
            .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt.id))
            .order_by(GoodsReceiptLine::POSITION)
            .get(&mut *tx)
            .await?;
        for line in lines.into_iter().filter(|l| l.accepted_qty_milli > 0) {
            found.push((receipt.clone(), line));
        }
    }
    Ok((found, None))
}

/// The weight of `line` on `basis`; `None` for a measured basis with no
/// positive weight given.
fn weight_of(basis: AllocationBasis, line: &GoodsReceiptLine, given: &[(i64, i64)]) -> Option<i64> {
    match basis {
        AllocationBasis::Value => Some(value_of(line.accepted_qty_milli, line.unit_cost)),
        AllocationBasis::Quantity => Some(line.accepted_qty_milli),
        AllocationBasis::Weight | AllocationBasis::Volume => given
            .iter()
            .find(|(id, _)| *id == line.id)
            .map(|(_, weight)| *weight)
            .filter(|weight| *weight > 0),
    }
}

/// What stands in the way of allocating `landed`, read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    landed: &LandedCost,
    asked: &AllocateInput,
    by: Option<i64>,
) -> Result<AllocateInput> {
    let mut input = asked.clone();
    input.by = by;
    input.problem = None;
    let lines = cost_lines_in(tx, landed.id).await?;
    let (found, problem) = accepted_lines(tx, landed.id).await?;
    if problem.is_some() {
        input.problem = problem;
    } else if found.is_empty() {
        input.problem = Some(said("wl_inventory.landed.no_receipts", &[]));
    } else if lines.is_empty() || lines.iter().map(|l| l.amount).sum::<i64>() == 0 {
        input.problem = Some(said("wl_inventory.landed.no_costs", &[]));
    } else if let Some((_, line)) = found
        .iter()
        .find(|(_, line)| weight_of(landed.basis, line, &input.weights).is_none())
    {
        input.problem = Some(said(
            "wl_inventory.landed.weight_needed",
            &[("line", &line.id.to_string())],
        ));
    } else if found
        .iter()
        .all(|(_, line)| weight_of(landed.basis, line, &input.weights) == Some(0))
    {
        input.problem = Some(said("wl_inventory.landed.no_receipts", &[]));
    }
    Ok(input)
}

fn input(ctx: &TransitionCtx) -> Option<AllocateInput> {
    ctx.input_as::<AllocateInput>().ok()
}

/// The effect of `allocate`: the allocation rows, the spread, the stock
/// re-valued and the shares stored.
async fn allocate_in(tx: &mut Transaction, landed: LandedCost, run: &AllocateInput) -> Result {
    let found = gather(tx, &landed, run, run.by).await?;
    if let Some(problem) = found.problem {
        return Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, problem));
    }
    let now = renox::db::now();
    let total: i64 = cost_lines_in(tx, landed.id)
        .await?
        .iter()
        .map(|l| l.amount)
        .sum();
    let (accepted, _) = accepted_lines(tx, landed.id).await?;
    let weights: Vec<i64> = accepted
        .iter()
        .map(|(_, line)| weight_of(landed.basis, line, &run.weights).unwrap_or(0))
        .collect();
    let amounts = spread(total, &weights);
    let mut rows = Vec::new();
    for (((receipt, line), weight), amount) in accepted.iter().zip(&weights).zip(&amounts) {
        let mut row = Allocation {
            landed_cost_id: landed.id,
            receipt_line_id: line.id,
            item_id: line.item_id,
            location_id: receipt.location_id,
            cost_object_type: receipt.scope_type.clone(),
            cost_object_id: receipt.scope_id,
            quantity_milli: line.accepted_qty_milli,
            weight: *weight,
            amount: *amount,
            ..Allocation::default()
        };
        row.insert(&mut *tx).await?;
        rows.push(row);
    }
    apply_shares(tx, landed.id, None, &rows, &amounts).await?;
    LandedCost::query()
        .where_(LandedCost::ID.eq(landed.id))
        .update(
            &mut *tx,
            &[
                ("allocated_by", &run.by),
                ("allocated_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for LandedCost {
    type State = LandedCostStatus;
    const COLUMN: renox::db::Col<LandedCost, LandedCostStatus> = LandedCost::STATUS;

    fn machine() -> Machine<Self> {
        use LandedCostStatus::{Allocated, Cancelled, Draft};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Allocated)
            .tone("success")
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(ALLOCATE)
                    .label("Allocate")
                    .tone("primary")
                    .permission(permissions::LANDED_COSTS_POST)
                    .from([Draft])
                    .to(Allocated)
                    .guard("allocation_allowed", |_landed: &LandedCost, ctx| {
                        let problem = input(ctx).map_or_else(
                            || Some(said("wl_inventory.landed.bad_input", &[])),
                            |input| input.problem,
                        );
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|landed: &LandedCost, ctx, tx| {
                        let landed = landed.clone();
                        let run = input(ctx).unwrap_or_default();
                        Box::pin(async move { allocate_in(tx, landed, &run).await })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .permission(permissions::LANDED_COSTS_POST)
                    .from([Draft])
                    .to(Cancelled),
            );
        crate::landed_adjust::add_to(machine)
    }
}

/// `user` allocates landed cost `id`. `weights` are the weight or volume of
/// receipt lines, for those bases.
///
/// # Errors
///
/// `Forbidden` without `landed_costs.post`; `Guard` (422) for a problem of
/// the landed cost.
pub async fn allocate(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    weights: &[(i64, i64)],
) -> std::result::Result<LandedCost, TransitionError> {
    let landed = LandedCost::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    let found = gather(
        &mut tx,
        &landed,
        &AllocateInput::weighted(weights),
        Some(user.id),
    )
    .await?;
    tx.rollback().await?;
    if let Some(problem) = &found.problem {
        return Err(TransitionError::Guard(problem.clone()));
    }
    let outcome = landed
        .transition(ALLOCATE)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    let _outcome = outcome;
    Ok(LandedCost::find_or_404(&state.db, id).await?)
}

/// The landed costs wl-invoicing may code a bill line to
/// (`wl_kernel::invoicing::LandedCosts`).
#[derive(Debug, Clone, Copy, Default)]
pub struct LandedCostBook;

impl wl_kernel::invoicing::LandedCosts for LandedCostBook {
    fn entity_of<'a>(
        &'a self,
        db: &'a Db,
        landed_cost: &'a Subject,
    ) -> wl_kernel::invoicing::SupplierBillFuture<'a, Option<i64>> {
        Box::pin(async move {
            if landed_cost.subject_type != LANDED_COST_SUBJECT_TYPE {
                return Ok(None);
            }
            Ok(LandedCost::find(db, landed_cost.subject_id)
                .await?
                .map(|landed| landed.legal_entity_id))
        })
    }
}

/// The shares by item, to read the split of an allocation or an adjustment.
pub fn by_item(shares: &[Share]) -> BTreeMap<i64, (i64, i64)> {
    let mut found = BTreeMap::new();
    for share in shares {
        let entry: &mut (i64, i64) = found.entry(share.item_id).or_default();
        entry.0 += share.revalued;
        entry.1 += share.variance;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remainder_goes_to_the_largest_share_and_the_first_on_a_tie() {
        // AED 1,000.00 over items of AED 1,000 and AED 2,000: 333.33 and
        // 666.67, the odd fil to the larger.
        assert_eq!(spread(100_000, &[100_000, 200_000]), [33_333, 66_667]);
        // A tie: the first.
        assert_eq!(spread(100, &[1, 1, 1]), [34, 33, 33]);
        // Nothing to spread over.
        assert_eq!(spread(100, &[0, 0]), [0, 0]);
        assert_eq!(spread(100, &[]), Vec::<i64>::new());
    }

    #[test]
    fn a_refund_spreads_the_same_way_with_the_sign() {
        assert_eq!(spread(-100_000, &[100_000, 200_000]), [-33_333, -66_667]);
        let shares = spread(-600_000, &[1, 2, 3]);
        assert_eq!(shares.iter().sum::<i64>(), -600_000);
    }

    #[test]
    fn the_shares_always_add_up_to_the_total() {
        for total in [1, 7, 99_999, 123_457] {
            let shares = spread(total, &[3, 5, 11, 2]);
            assert_eq!(shares.iter().sum::<i64>(), total);
        }
    }

    #[test]
    fn the_part_on_goods_still_on_hand_revalues_and_the_rest_is_consumed() {
        // 100 received, 60 on hand, AED 1,000: AED 600 and AED 400.
        assert_eq!(split(100_000, 100_000, 60_000), (60_000, 40_000));
        // More on hand than was received counts as all of it.
        assert_eq!(split(100_000, 100_000, 250_000), (100_000, 0));
        // Nothing on hand: all of it is consumed.
        assert_eq!(split(100_000, 100_000, 0), (0, 100_000));
        // The consumed part is rounded; the on-hand part takes the remainder.
        assert_eq!(split(100, 3_000, 1_000), (33, 67));
        // A refund splits the same way.
        assert_eq!(split(-100_000, 100_000, 60_000), (-60_000, -40_000));
    }

    #[test]
    fn the_landed_cost_machine_allocates_adjusts_and_cancels() {
        let machine = LandedCost::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let allocate = machine.find(ALLOCATE).expect("allocate");
        assert_eq!(allocate.from_states(), ["draft"]);
        assert_eq!(allocate.to_state(), Some("allocated"));
        assert!(!allocate.is_system());
        let cancel = machine.find(CANCEL).expect("cancel");
        assert_eq!(cancel.to_state(), Some("cancelled"));
    }

    #[test]
    fn a_basis_is_measured_for_weight_and_volume_only() {
        assert!(AllocationBasis::Weight.is_measured());
        assert!(AllocationBasis::Volume.is_measured());
        assert!(!AllocationBasis::Value.is_measured());
        assert!(!AllocationBasis::Quantity.is_measured());
    }

    #[test]
    fn rows_of_one_item_at_one_place_are_one_part() {
        let row = |item, location, quantity| Allocation {
            item_id: item,
            location_id: location,
            quantity_milli: quantity,
            ..Allocation::default()
        };
        let found = parts(
            &[row(1, 1, 10_000), row(2, 1, 5_000), row(1, 1, 20_000)],
            &[100, 50, 200],
        );
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].amount, found[0].received_milli), (300, 30_000));
        assert_eq!((found[1].amount, found[1].received_milli), (50, 5_000));
    }
}
