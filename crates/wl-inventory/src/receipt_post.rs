//! Posting a goods receipt (wl-inventory §4 *Goods receipt* `post`, §5.1
//! step 4, §5.6 steps 1 and 2, `STK.ACL.04`).
//!
//! The storekeeper has recorded, per line, what was received and how much of
//! it is accepted and rejected. `post` (a holder of `goods_receipts.post` in
//! the scope of the receiving location) moves the receipt from `draft` to
//! `posted`:
//!
//! - the guards: the source is an open purchase order and every line names a
//!   line of it; the receipt has lines, none negative, and something was
//!   received; per line accepted plus rejected equals received exactly; a
//!   rejected quantity has a configured reason, says whether it was refused at
//!   the door or kept for collection, and carries the reading or photo the
//!   reason requires; an accepted quantity of a lot-tracked item has its lot
//!   number and expiry date;
//! - the effect, in the transaction: a `receive` movement for the accepted
//!   quantity only (and a lot, `quarantine` or `released` as the item says),
//!   the item's moving weighted average cost, the purchase order's `receive`
//!   (only accepted quantity counts, rejected quantity stays outstanding),
//!   and a Return to supplier for the rejected lines (one for the goods
//!   refused at the door, dispatched at once by the system, and a `draft` for
//!   the goods kept for collection; rejected goods never get a movement);
//! - `GoodsReceived` after the commit ([`crate::events`]).
//!
//! An accepted quantity above the ordered one by more than the tolerance
//! (§7, 5%) is not refused: the user's `post` becomes an approval request
//! (`PLT.APR.01`) for a holder of `purchase_orders.issue` in the location's
//! scope, never the poster, and the receipt stays `draft`. When it is
//! approved the kernel runs `post_approved` (the same guards and effect, run
//! by the system with the payload of the request); a rejection leaves the
//! receipt a draft for correction. The policy's condition reads
//! `percentage_bps`, the largest over-receipt of the receipt, from the
//! transition's input.
//!
//! A receipt may instead be a free replacement against a `dispatched` Return
//! to supplier (issue #225, [`crate::goods_receipt::create_replacement_draft`]):
//! no purchase order, so nothing is received on an order or left to match in
//! wl-invoicing. The guard reads the return: each line names a `replace`
//! line of it that has no order line, or whose order is `closed` (a
//! replacement with an open order line is received on the order), of the
//! same item, and what was received of a return line never goes above what is
//! still to be replaced (the returned quantity less what earlier receipts
//! covered). The effect receives the accepted quantity at zero cost, so the
//! item's average cost is diluted, counts it as covered on the return line,
//! sends what was rejected again to a new return (as for an order), and
//! credits the return when every line is covered. `SupplierReturnCredited`
//! follows the receipt's `GoodsReceived`.
//!
//! Decisions: the guards read what [`prepare`] gathered into a
//! [`PostInput`], as the guards of the other machines do, and the effect reads
//! the lines again in its own transaction. The nested `receive` of the
//! purchase order emits no `Transitioned<PurchaseOrder>` yet (nothing
//! listens).

use std::collections::{BTreeMap, BTreeSet};

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{
    Machine, Outcome, StateMachine, Transition, TransitionCtx, TransitionError,
};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::numbering::{Reset, Series};

use crate::config::InventoryConfig;
use crate::config_of;
use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use crate::item::{Item, value_of, weighted_average};
use crate::lot::{Lot, LotStatus};
use crate::movement::{self, StockMovement};
use crate::permissions;
use crate::purchase_order::{
    PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, ReceiveInput, ReceiveLine,
    over_receipt_bps,
};
use crate::supplier_return::{
    RejectedHandling, ReturnOrigin, ReturnStatus, SupplierReturn, SupplierReturnLine, WantedOutcome,
};
use crate::texts::said;

/// The `post` transition's name.
pub const POST: &str = "post";

/// The system's `post_approved` transition's name: the kernel runs it when
/// the approval of an over-receipt is given.
pub const POST_APPROVED: &str = "post_approved";

/// The approval policy's action key of `post` (`STK.ACL.04`, `PLT.APR.01`).
pub const ACTION: &str = "inventory.goods_receipt_post";

/// The numbers of goods receipts: `GRN-2026-0001`.
pub const GOODS_RECEIPT_SERIES: Series = Series {
    key: "goods_receipt",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "GRN",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// The numbers of returns to supplier: `RTS-2026-0001`.
pub const SUPPLIER_RETURN_SERIES: Series = Series {
    key: "supplier_return",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "RTS",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// What a `post` run carries to its guards and effect, gathered by
/// [`prepare`]. Each `*_problem` is the text of the first thing wrong of
/// that kind, `None` when nothing is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PostInput {
    /// The largest over-receipt of the receipt, in basis points of the
    /// ordered quantity: what the approval policy's condition reads.
    pub percentage_bps: i64,
    /// The tolerance the order checks, in basis points.
    pub tolerance_bps: i64,
    /// The user who posts it (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The receipt has lines, and something was received.
    pub lines_problem: Option<String>,
    /// The order is open and every line is on it.
    pub source_problem: Option<String>,
    /// Accepted plus rejected equals received on every line.
    pub quantities_problem: Option<String>,
    /// A rejected quantity has its reason, handling and evidence.
    pub rejections_problem: Option<String>,
    /// An accepted lot-tracked line has its lot and expiry.
    pub lots_problem: Option<String>,
}

impl PostInput {
    /// The first problem, in the order the guards run.
    pub fn first_problem(&self) -> Option<&str> {
        [
            &self.lines_problem,
            &self.source_problem,
            &self.quantities_problem,
            &self.rejections_problem,
            &self.lots_problem,
        ]
        .into_iter()
        .find_map(|problem| problem.as_deref())
    }
}

/// What [`check`] looks at.
pub struct Loaded<'a> {
    /// The receipt.
    pub receipt: &'a GoodsReceipt,
    /// Its lines.
    pub lines: &'a [GoodsReceiptLine],
    /// The items of the lines, by id.
    pub items: &'a BTreeMap<i64, Item>,
    /// The purchase order, if any.
    pub order: Option<&'a PurchaseOrder>,
    /// The lines of the order.
    pub order_lines: &'a [PurchaseOrderLine],
    /// The Return to supplier a free replacement is received against.
    pub source_return: Option<&'a SupplierReturn>,
    /// The lines of that return.
    pub return_lines: &'a [SupplierReturnLine],
    /// The return lines whose purchase order is not `closed`: a replacement
    /// is received on the order, not on the return.
    pub open_order_lines: &'a BTreeSet<i64>,
    /// The settings.
    pub config: &'a InventoryConfig,
}

/// The problems of posting `loaded`, and the over-receipt that decides on
/// the approval. Pure: nothing is read or written.
pub fn check(loaded: &Loaded<'_>) -> PostInput {
    let mut input = PostInput {
        tolerance_bps: loaded.config.over_receipt_tolerance_bps,
        ..PostInput::default()
    };
    input.lines_problem = lines_problem(loaded);
    input.source_problem = source_problem(loaded);
    input.quantities_problem = quantities_problem(loaded);
    input.rejections_problem = rejections_problem(loaded);
    input.lots_problem = lots_problem(loaded);
    input.percentage_bps = largest_over_receipt(loaded);
    input
}

fn lines_problem(loaded: &Loaded<'_>) -> Option<String> {
    if loaded.lines.is_empty() {
        return Some(said("wl_inventory.receipt.no_lines", &[]));
    }
    if loaded.lines.iter().all(|line| line.received_qty_milli == 0) {
        return Some(said("wl_inventory.receipt.nothing_received", &[]));
    }
    None
}

/// The problem of a free replacement received against a return.
fn return_problem(loaded: &Loaded<'_>, ret: &SupplierReturn) -> Option<String> {
    if ret.status != ReturnStatus::Dispatched {
        return Some(said("wl_inventory.receipt.return_not_dispatched", &[]));
    }
    let mut asked: BTreeMap<i64, i64> = BTreeMap::new();
    for line in loaded.lines {
        let position = line.position.to_string();
        let on_return = line
            .return_line_id
            .and_then(|id| loaded.return_lines.iter().find(|l| l.id == id));
        let Some(return_line) = on_return else {
            return Some(said(
                "wl_inventory.receipt.line_not_on_return",
                &[("position", &position)],
            ));
        };
        if return_line.item_id != line.item_id {
            return Some(said(
                "wl_inventory.receipt.item_differs",
                &[("position", &position)],
            ));
        }
        if return_line.wanted_outcome != WantedOutcome::Replace {
            return Some(said(
                "wl_inventory.receipt.return_line_not_replace",
                &[("position", &position)],
            ));
        }
        if loaded.open_order_lines.contains(&return_line.id) {
            return Some(said(
                "wl_inventory.receipt.return_line_on_open_order",
                &[("position", &position)],
            ));
        }
        *asked.entry(return_line.id).or_insert(0) += line.received_qty_milli;
    }
    for (id, received) in asked {
        let return_line = loaded.return_lines.iter().find(|l| l.id == id)?;
        let remaining = (return_line.quantity_milli - return_line.covered_milli).max(0);
        if received > remaining {
            return Some(said(
                "wl_inventory.receipt.above_returned",
                &[
                    ("position", &return_line.position.to_string()),
                    ("remaining", &crate::item::quantity_text(remaining)),
                    ("received", &crate::item::quantity_text(received)),
                ],
            ));
        }
    }
    None
}

fn source_problem(loaded: &Loaded<'_>) -> Option<String> {
    if loaded.receipt.supplier_return_id.is_some() {
        return match loaded.source_return {
            Some(ret) => return_problem(loaded, ret),
            None => Some(said("wl_inventory.receipt.unknown_return", &[])),
        };
    }
    let Some(order) = loaded.order else {
        return Some(said("wl_inventory.receipt.no_order", &[]));
    };
    if !order.is_open_for_receiving() {
        return Some(said(
            "wl_inventory.receipt.order_not_open",
            &[("status", order.status.as_str())],
        ));
    }
    for line in loaded.lines {
        let on_order = line
            .po_line_id
            .and_then(|id| loaded.order_lines.iter().find(|l| l.id == id));
        match on_order {
            None => {
                return Some(said(
                    "wl_inventory.receipt.line_not_on_order",
                    &[("position", &line.position.to_string())],
                ));
            }
            Some(po_line) if po_line.item_id != line.item_id => {
                return Some(said(
                    "wl_inventory.receipt.item_differs",
                    &[("position", &line.position.to_string())],
                ));
            }
            Some(_) => {}
        }
    }
    None
}

fn quantities_problem(loaded: &Loaded<'_>) -> Option<String> {
    for line in loaded.lines {
        let position = line.position.to_string();
        if line.received_qty_milli < 0 || line.accepted_qty_milli < 0 || line.rejected_qty_milli < 0
        {
            return Some(said(
                "wl_inventory.receipt.negative",
                &[("position", &position)],
            ));
        }
        if line.accepted_qty_milli + line.rejected_qty_milli != line.received_qty_milli {
            return Some(said(
                "wl_inventory.receipt.not_balanced",
                &[("position", &position)],
            ));
        }
    }
    None
}

fn rejections_problem(loaded: &Loaded<'_>) -> Option<String> {
    for line in loaded
        .lines
        .iter()
        .filter(|line| line.rejected_qty_milli > 0)
    {
        let position = line.position.to_string();
        let Some(code) = line
            .rejection_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        else {
            return Some(said(
                "wl_inventory.receipt.reason_needed",
                &[("position", &position)],
            ));
        };
        let Some(reason) = loaded.config.reason(code) else {
            return Some(said(
                "wl_inventory.receipt.reason_unknown",
                &[("position", &position), ("reason", code)],
            ));
        };
        if line.rejected_handling.is_none() {
            return Some(said(
                "wl_inventory.receipt.handling_needed",
                &[("position", &position)],
            ));
        }
        let has_evidence = line
            .evidence
            .as_deref()
            .is_some_and(|evidence| !evidence.trim().is_empty());
        if reason.evidence_required && !has_evidence {
            return Some(said(
                "wl_inventory.receipt.evidence_needed",
                &[("position", &position), ("reason", code)],
            ));
        }
    }
    None
}

fn lots_problem(loaded: &Loaded<'_>) -> Option<String> {
    for line in loaded
        .lines
        .iter()
        .filter(|line| line.accepted_qty_milli > 0)
    {
        let Some(item) = loaded.items.get(&line.item_id) else {
            return Some(said(
                "wl_inventory.receipt.unknown_item",
                &[("position", &line.position.to_string())],
            ));
        };
        let has_lot = line
            .lot_number
            .as_deref()
            .is_some_and(|number| !number.trim().is_empty());
        if item.lot_tracked && (!has_lot || line.expiry_date.is_none()) {
            return Some(said(
                "wl_inventory.receipt.lot_needed",
                &[
                    ("position", &line.position.to_string()),
                    ("item", &item.code),
                ],
            ));
        }
    }
    None
}

/// The largest over-receipt of the receipt, in basis points: what the
/// accepted quantity of an order line would be after the receipt, against
/// what was ordered.
fn largest_over_receipt(loaded: &Loaded<'_>) -> i64 {
    let mut added: BTreeMap<i64, i64> = BTreeMap::new();
    for line in loaded.lines {
        if let Some(id) = line.po_line_id {
            *added.entry(id).or_insert(0) += line.accepted_qty_milli;
        }
    }
    added
        .into_iter()
        .filter_map(|(id, add)| {
            let po_line = loaded.order_lines.iter().find(|l| l.id == id)?;
            Some(over_receipt_bps(
                po_line.quantity_milli,
                po_line.accepted_qty_milli + add,
            ))
        })
        .max()
        .unwrap_or(0)
}

fn input(ctx: &TransitionCtx) -> Option<PostInput> {
    ctx.input_as::<PostInput>().ok()
}

/// A guard that refuses with the problem `pick` finds in the run's input.
fn with_problem(
    transition: Transition<GoodsReceipt>,
    name: &'static str,
    pick: fn(&PostInput) -> Option<String>,
) -> Transition<GoodsReceipt> {
    transition.guard(name, move |_receipt: &GoodsReceipt, ctx| {
        let problem = input(ctx).map_or_else(
            || Some(said("wl_inventory.receipt.no_lines", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// The guards and the effect `post` and `post_approved` share.
fn rules(transition: Transition<GoodsReceipt>, approved: bool) -> Transition<GoodsReceipt> {
    let transition = transition
        .from([GoodsReceiptStatus::Draft])
        .to(GoodsReceiptStatus::Posted);
    let transition = with_problem(transition, "has_lines", |i| i.lines_problem.clone());
    let transition = with_problem(transition, "source_is_an_open_order", |i| {
        i.source_problem.clone()
    });
    let transition = with_problem(transition, "quantities_balance", |i| {
        i.quantities_problem.clone()
    });
    let transition = with_problem(transition, "rejections_explained", |i| {
        i.rejections_problem.clone()
    });
    let transition = with_problem(transition, "lots_and_expiry", |i| i.lots_problem.clone());
    transition.effect(move |receipt: &GoodsReceipt, ctx, tx| {
        let receipt = receipt.clone();
        let input = input(ctx).unwrap_or_default();
        Box::pin(async move { post_in(tx, receipt, &input, approved).await })
    })
}

/// `post` (a holder of `goods_receipts.post` in the receipt's scope) and the
/// system's `post_approved`.
pub(crate) fn transitions() -> Vec<Transition<GoodsReceipt>> {
    vec![
        rules(
            Transition::new(POST)
                .label("Post")
                .tone("primary")
                .permission_in(
                    permissions::GOODS_RECEIPTS_POST,
                    |receipt: &GoodsReceipt| receipt.scope(),
                ),
            false,
        ),
        rules(
            Transition::new(POST_APPROVED)
                .label("Post (approved)")
                .tone("primary")
                .system(),
            true,
        ),
    ]
}

impl StateMachine for GoodsReceipt {
    type State = GoodsReceiptStatus;
    const COLUMN: renox::db::Col<GoodsReceipt, GoodsReceiptStatus> = GoodsReceipt::STATUS;

    fn machine() -> Machine<Self> {
        use GoodsReceiptStatus::{Draft, Posted, Reversed};
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Posted)
            .tone("success")
            .state(Reversed)
            .tone("neutral")
            .terminal();
        transitions()
            .into_iter()
            .fold(machine, Machine::transition)
            .transition(crate::receipt_reverse::transition())
    }
}

/// The lot of `line` for `item`: the one with that number, else a new one in
/// `quarantine` or `released` as the item says.
async fn ensure_lot(
    tx: &mut Transaction,
    item: &Item,
    line: &GoodsReceiptLine,
    receipt: &GoodsReceipt,
) -> Result<i64> {
    let number = line.lot_number.clone().unwrap_or_default();
    let found = Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .where_(Lot::NUMBER.eq(number.as_str()))
        .first(&mut *tx)
        .await?;
    if let Some(lot) = found {
        return Ok(lot.id);
    }
    let source = receipt.subject();
    let mut lot = Lot {
        item_id: item.id,
        number,
        source_type: Some(source.subject_type),
        source_id: Some(source.subject_id),
        expiry_date: line.expiry_date,
        status: if item.quarantine_new_lots {
            LotStatus::Quarantine
        } else {
            LotStatus::Released
        },
        ..Lot::default()
    };
    lot.insert(&mut *tx).await?;
    Ok(lot.id)
}

/// A Return to supplier for `lines` of `receipt`, rejected for the reasons on
/// the lines, in `draft`; the caller dispatches a door-refused one (§4).
async fn make_return(
    tx: &mut Transaction,
    receipt: &GoodsReceipt,
    handling: RejectedHandling,
    lines: &[&GoodsReceiptLine],
    by: Option<i64>,
) -> Result<SupplierReturn> {
    let number = SUPPLIER_RETURN_SERIES
        .next(&mut *tx, None, None, today())
        .await?;
    let proof =
        (handling == RejectedHandling::RefusedAtDoor).then(|| receipt.subject().to_string());
    let source = receipt.subject();
    let mut ret = SupplierReturn {
        number: Some(number),
        supplier_party_id: receipt.supplier_party_id,
        source_type: source.subject_type,
        source_id: source.subject_id,
        handling,
        collection_proof: proof,
        location_id: Some(receipt.location_id),
        status: ReturnStatus::Draft,
        created_by: by,
        ..SupplierReturn::default()
    };
    ret.insert(&mut *tx).await?;
    for (position, line) in (1..).zip(lines) {
        let mut row = SupplierReturnLine {
            supplier_return_id: ret.id,
            position,
            item_id: line.item_id,
            lot_id: None,
            quantity_milli: line.rejected_qty_milli,
            unit_cost: line.unit_cost,
            reason: line.rejection_reason.clone().unwrap_or_default(),
            wanted_outcome: line.wanted_outcome,
            po_line_id: line.po_line_id,
            origin: ReturnOrigin::RejectedAtReceipt,
            receipt_line_id: Some(line.id),
            ..SupplierReturnLine::default()
        };
        row.insert(&mut *tx).await?;
    }
    Ok(ret)
}

/// The lines with a rejected quantity that `handling` says what became of.
fn rejected_with(lines: &[GoodsReceiptLine], handling: RejectedHandling) -> Vec<&GoodsReceiptLine> {
    lines
        .iter()
        .filter(|line| line.rejected_qty_milli > 0 && line.rejected_handling == Some(handling))
        .collect()
}

/// The effect of `post` and `post_approved`, in the run's transaction.
async fn post_in(
    tx: &mut Transaction,
    receipt: GoodsReceipt,
    input: &PostInput,
    approved: bool,
) -> Result {
    let now = renox::db::now();
    let number = GOODS_RECEIPT_SERIES
        .next(&mut *tx, None, None, today())
        .await?;
    let lines = GoodsReceiptLine::query()
        .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt.id))
        .order_by(GoodsReceiptLine::POSITION)
        .get(&mut *tx)
        .await?;
    let source = receipt.subject();

    // Accepted quantity only enters stock.
    for line in lines.iter().filter(|line| line.accepted_qty_milli > 0) {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let lot_id = if item.lot_tracked {
            Some(ensure_lot(tx, &item, line, &receipt).await?)
        } else {
            None
        };
        let on_hand = movement::on_hand_everywhere_in(tx, item.id).await?;
        let value = value_of(line.accepted_qty_milli, line.unit_cost);
        let mut moved = StockMovement::receive(
            item.id,
            lot_id,
            receipt.location_id,
            line.accepted_qty_milli,
            line.unit_cost,
            value,
            &source,
            input.by,
        );
        moved.insert(&mut *tx).await?;
        let average = weighted_average(
            on_hand,
            item.average_cost,
            line.accepted_qty_milli,
            line.unit_cost,
        );
        Item::query()
            .where_(Item::ID.eq(item.id))
            .update(
                &mut *tx,
                &[("average_cost", &average), ("updated_at", &now)],
            )
            .await?;
        if lot_id.is_some() {
            GoodsReceiptLine::query()
                .where_(GoodsReceiptLine::ID.eq(line.id))
                .update(&mut *tx, &[("lot_id", &lot_id), ("updated_at", &now)])
                .await?;
        }
    }

    // A replacement is covered on its return line, not received on an order.
    if let Some(return_id) = receipt.supplier_return_id {
        cover_return_lines(tx, return_id, &lines).await?;
    }

    // The order counts the accepted quantity; a receipt that accepts nothing
    // leaves it as it is.
    if let Some(order_id) = receipt.purchase_order_id {
        receive_on_order(tx, order_id, &lines, input, approved).await?;
    }

    // Rejected goods never enter stock: they go back to the supplier.
    for handling in [
        RejectedHandling::RefusedAtDoor,
        RejectedHandling::KeptForCollection,
    ] {
        let group = rejected_with(&lines, handling);
        if group.is_empty() {
            continue;
        }
        let ret = make_return(tx, &receipt, handling, &group, input.by).await?;
        // The goods left with the driver already: dispatched at once by the
        // system, with the receipt as the collection proof (BL-STK-03).
        if handling == RejectedHandling::RefusedAtDoor {
            crate::return_dispatch::dispatch_at_door(tx, &ret, input.by).await?;
        }
    }

    // Every line replaced: the return is credited. The event follows the
    // receipt's (`crate::events`).
    if let Some(return_id) = receipt.supplier_return_id {
        let ret = SupplierReturn::find_or_404(&mut *tx, return_id).await?;
        let _outcome =
            crate::return_credit::credit_if_covered(tx, &ret, "replacement", None).await?;
    }

    // Not the status column, so outside the machine's guard.
    GoodsReceipt::query()
        .where_(GoodsReceipt::ID.eq(receipt.id))
        .update(
            &mut *tx,
            &[
                ("number", &Some(number)),
                ("posted_by", &input.by),
                ("posted_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// Adds the accepted quantity of each line of a replacement receipt to the
/// covered quantity of the return line it replaces.
async fn cover_return_lines(
    tx: &mut Transaction,
    return_id: i64,
    lines: &[GoodsReceiptLine],
) -> Result {
    let now = renox::db::now();
    for line in lines.iter().filter(|line| line.accepted_qty_milli > 0) {
        let Some(return_line_id) = line.return_line_id else {
            continue;
        };
        let return_line = SupplierReturnLine::find_or_404(&mut *tx, return_line_id).await?;
        if return_line.supplier_return_id != return_id {
            continue;
        }
        let covered = return_line.covered_milli + line.accepted_qty_milli;
        SupplierReturnLine::query()
            .where_(SupplierReturnLine::ID.eq(return_line.id))
            .update(
                &mut *tx,
                &[("covered_milli", &covered), ("updated_at", &now)],
            )
            .await?;
    }
    Ok(())
}

/// The order's `receive` for the accepted quantities of `lines`.
async fn receive_on_order(
    tx: &mut Transaction,
    order_id: i64,
    lines: &[GoodsReceiptLine],
    input: &PostInput,
    approved: bool,
) -> Result {
    let order = PurchaseOrder::find_or_404(&mut *tx, order_id).await?;
    let order_lines = PurchaseOrderLine::query()
        .where_(PurchaseOrderLine::PURCHASE_ORDER_ID.eq(order_id))
        .order_by(PurchaseOrderLine::POSITION)
        .get(&mut *tx)
        .await?;
    let receive = ReceiveInput {
        lines: order_lines
            .iter()
            .map(|po_line| ReceiveLine {
                po_line_id: po_line.id,
                position: po_line.position,
                ordered_milli: po_line.quantity_milli,
                accepted_before_milli: po_line.accepted_qty_milli,
                add_milli: lines
                    .iter()
                    .filter(|line| line.po_line_id == Some(po_line.id))
                    .map(|line| line.accepted_qty_milli)
                    .sum(),
            })
            .collect(),
        tolerance_bps: input.tolerance_bps,
        approved,
    };
    if receive.lines.iter().all(|line| line.add_milli == 0) {
        return Ok(());
    }
    // Nothing listens to the order yet: the outcome is not emitted.
    let _outcome = order
        .transition(receive.transition())
        .by_system()
        .input(&receive)
        .run_in(&mut *tx)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// Reads what the guards of `post` need, as `user`.
///
/// # Errors
///
/// A database error.
pub async fn prepare(
    state: &AppState,
    receipt: &GoodsReceipt,
    by: Option<i64>,
) -> Result<PostInput> {
    let lines = crate::goods_receipt::lines_of(&state.db, receipt.id).await?;
    let mut items = BTreeMap::new();
    for line in &lines {
        if let Some(item) = Item::find(&state.db, line.item_id).await? {
            items.insert(item.id, item);
        }
    }
    let order = match receipt.purchase_order_id {
        Some(id) => PurchaseOrder::find(&state.db, id).await?,
        None => None,
    };
    let order_lines = match &order {
        Some(order) => crate::purchase_order::lines_of(&state.db, order.id).await?,
        None => Vec::new(),
    };
    let source_return = match receipt.supplier_return_id {
        Some(id) => SupplierReturn::find(&state.db, id).await?,
        None => None,
    };
    let return_lines = match &source_return {
        Some(ret) => crate::supplier_return::lines_of(&state.db, ret.id).await?,
        None => Vec::new(),
    };
    let mut open_order_lines = BTreeSet::new();
    for return_line in &return_lines {
        let Some(po_line_id) = return_line.po_line_id else {
            continue;
        };
        if let Some(po_line) = PurchaseOrderLine::find(&state.db, po_line_id).await?
            && let Some(order) = PurchaseOrder::find(&state.db, po_line.purchase_order_id).await?
            && order.status != PurchaseOrderStatus::Closed
        {
            open_order_lines.insert(return_line.id);
        }
    }
    let config = config_of(state);
    let mut input = check(&Loaded {
        receipt,
        lines: &lines,
        items: &items,
        order: order.as_ref(),
        order_lines: &order_lines,
        source_return: source_return.as_ref(),
        return_lines: &return_lines,
        open_order_lines: &open_order_lines,
        config: &config,
    });
    input.by = by;
    Ok(input)
}

/// What [`post`] did.
#[derive(Debug, Clone)]
pub enum PostOutcome {
    /// The receipt is `posted`: stock, the order and the returns are done.
    Posted(GoodsReceipt),
    /// The accepted quantity is above the tolerance: the receipt stays
    /// `draft` and waits for the approval request (`STK.ACL.04`).
    Waiting(ApprovalRequest),
}

/// `user` posts receipt `receipt_id`. Refused with the first problem before
/// anything is asked or written; above the tolerance it becomes an approval
/// request instead (see the module docs).
///
/// # Errors
///
/// `Forbidden` without `goods_receipts.post` in the receipt's scope;
/// `NotFrom` unless `draft`; `Guard` (422) for a problem of the receipt.
pub async fn post(
    state: &AppState,
    user: &AuthUser,
    receipt_id: i64,
) -> std::result::Result<PostOutcome, TransitionError> {
    let receipt = GoodsReceipt::find_or_404(&state.db, receipt_id).await?;
    if receipt.status != GoodsReceiptStatus::Draft {
        return Err(TransitionError::Guard(said(
            "wl_inventory.receipt.not_draft",
            &[],
        )));
    }
    // The goods arrived when posting was asked for, even if an approval
    // comes days later (the supplier score counts this day).
    let receipt = if receipt.arrived_at.is_none() {
        let arrived = Some(renox::db::now());
        GoodsReceipt::query()
            .where_(GoodsReceipt::ID.eq(receipt.id))
            .update(&state.db, &[("arrived_at", &arrived)])
            .await?;
        GoodsReceipt::find_or_404(&state.db, receipt.id).await?
    } else {
        receipt
    };
    let input = prepare(state, &receipt, Some(user.id)).await?;
    if let Some(problem) = input.first_problem() {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    let outcome = receipt
        .transition(POST)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    approvals::announce(state, &outcome).await?;
    match outcome {
        Outcome::Done(_) => Ok(PostOutcome::Posted(
            GoodsReceipt::find_or_404(&state.db, receipt.id).await?,
        )),
        Outcome::Pending(_) => {
            let request = ApprovalRequest::pending_about(&state.db, &receipt.subject())
                .await?
                .pop()
                .ok_or(Error::NotFound)?;
            Ok(PostOutcome::Waiting(request))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn item(id: i64, lot_tracked: bool) -> Item {
        Item {
            id,
            code: format!("ITEM-{id}"),
            lot_tracked,
            ..Item::default()
        }
    }

    fn order(status: PurchaseOrderStatus) -> PurchaseOrder {
        PurchaseOrder {
            id: 3,
            status,
            ..PurchaseOrder::default()
        }
    }

    fn po_line(id: i64, item_id: i64, ordered: i64, accepted: i64) -> PurchaseOrderLine {
        PurchaseOrderLine {
            id,
            purchase_order_id: 3,
            position: id,
            item_id,
            quantity_milli: ordered,
            unit_price: 1_200,
            accepted_qty_milli: accepted,
            ..PurchaseOrderLine::default()
        }
    }

    fn line(
        position: i64,
        po_line: i64,
        item_id: i64,
        received: i64,
        accepted: i64,
    ) -> GoodsReceiptLine {
        GoodsReceiptLine {
            id: position,
            position,
            po_line_id: Some(po_line),
            item_id,
            received_qty_milli: received,
            accepted_qty_milli: accepted,
            rejected_qty_milli: received - accepted,
            unit_cost: 1_200,
            ..GoodsReceiptLine::default()
        }
    }

    /// The input for `lines` on an issued order with one line of 100 kg.
    fn input_of(lines: Vec<GoodsReceiptLine>, tracked: bool) -> PostInput {
        let items = BTreeMap::from([(2, item(2, tracked))]);
        let order = order(PurchaseOrderStatus::Issued);
        let order_lines = vec![po_line(1, 2, 100_000, 0)];
        check(&Loaded {
            receipt: &GoodsReceipt::default(),
            lines: &lines,
            items: &items,
            order: Some(&order),
            order_lines: &order_lines,
            source_return: None,
            return_lines: &[],
            open_order_lines: &BTreeSet::new(),
            config: &InventoryConfig::default(),
        })
    }

    #[test]
    fn a_balanced_receipt_has_no_problem() {
        let input = input_of(vec![line(1, 1, 2, 100_000, 100_000)], false);
        assert_eq!(input.first_problem(), None);
        assert_eq!(input.percentage_bps, 0);
        assert_eq!(input.tolerance_bps, 500);
    }

    #[test]
    fn accepted_plus_rejected_must_equal_received() {
        let mut bad = line(1, 1, 2, 100_000, 80_000);
        bad.rejected_qty_milli = 10_000;
        let input = input_of(vec![bad], false);
        assert!(
            input
                .quantities_problem
                .is_some_and(|m| m.contains("accepted plus rejected"))
        );
        let mut negative = line(1, 1, 2, 100_000, 100_000);
        negative.accepted_qty_milli = -1;
        assert!(input_of(vec![negative], false).quantities_problem.is_some());
    }

    #[test]
    fn a_rejected_quantity_needs_its_reason_handling_and_evidence() {
        let mut rejected = line(1, 1, 2, 100_000, 80_000);
        let input = input_of(vec![rejected.clone()], false);
        assert!(
            input
                .rejections_problem
                .is_some_and(|m| m.contains("reason"))
        );
        rejected.rejection_reason = Some("spoiled".into());
        rejected.rejected_handling = Some(RejectedHandling::RefusedAtDoor);
        let input = input_of(vec![rejected.clone()], false);
        assert!(
            input
                .rejections_problem
                .is_some_and(|m| m.contains("not a rejection reason"))
        );
        rejected.rejection_reason = Some("temperature".into());
        rejected.rejected_handling = None;
        let input = input_of(vec![rejected.clone()], false);
        assert!(
            input
                .rejections_problem
                .is_some_and(|m| m.contains("refused at the door"))
        );
        rejected.rejected_handling = Some(RejectedHandling::KeptForCollection);
        assert_eq!(
            input_of(vec![rejected.clone()], false).rejections_problem,
            None
        );
        // A vertical may require a reading for a reason.
        let mut config = InventoryConfig::default();
        config
            .rejection_reasons
            .iter_mut()
            .find(|r| r.code == "temperature")
            .unwrap()
            .evidence_required = true;
        let lines = vec![rejected.clone()];
        let items = BTreeMap::from([(2, item(2, false))]);
        let order = order(PurchaseOrderStatus::Issued);
        let order_lines = vec![po_line(1, 2, 100_000, 0)];
        let loaded = Loaded {
            receipt: &GoodsReceipt::default(),
            lines: &lines,
            items: &items,
            order: Some(&order),
            order_lines: &order_lines,
            source_return: None,
            return_lines: &[],
            open_order_lines: &BTreeSet::new(),
            config: &config,
        };
        assert!(
            check(&loaded)
                .rejections_problem
                .is_some_and(|m| m.contains("reading or a photo"))
        );
        rejected.evidence = Some("9 C".into());
        let lines = vec![rejected];
        let loaded = Loaded {
            lines: &lines,
            ..loaded
        };
        assert_eq!(check(&loaded).rejections_problem, None);
    }

    #[test]
    fn a_lot_tracked_item_needs_its_lot_and_expiry() {
        let mut tracked = line(1, 1, 2, 100_000, 100_000);
        assert!(input_of(vec![tracked.clone()], true).lots_problem.is_some());
        tracked.lot_number = Some("L-1".into());
        assert!(input_of(vec![tracked.clone()], true).lots_problem.is_some());
        tracked.expiry_date = NaiveDate::from_ymd_opt(2027, 1, 31);
        assert_eq!(input_of(vec![tracked.clone()], true).lots_problem, None);
        // Nothing accepted: no lot is made, so none is needed.
        let none = line(1, 1, 2, 100_000, 0);
        let mut rejected = none;
        rejected.rejection_reason = Some("quality".into());
        rejected.rejected_handling = Some(RejectedHandling::RefusedAtDoor);
        assert_eq!(input_of(vec![rejected], true).lots_problem, None);
    }

    #[test]
    fn the_source_is_an_open_order_and_every_line_is_on_it() {
        let lines = vec![line(1, 1, 2, 10_000, 10_000)];
        let items = BTreeMap::from([(2, item(2, false))]);
        let order_lines = vec![po_line(1, 2, 100_000, 0)];
        let config = InventoryConfig::default();
        let receipt = GoodsReceipt::default();
        let loaded = |order: Option<&PurchaseOrder>, lines: &[GoodsReceiptLine]| {
            check(&Loaded {
                receipt: &receipt,
                lines,
                items: &items,
                order,
                order_lines: &order_lines,
                source_return: None,
                return_lines: &[],
                open_order_lines: &BTreeSet::new(),
                config: &config,
            })
            .source_problem
        };
        assert!(loaded(None, &lines).is_some_and(|m| m.contains("purchase order")));
        let draft = order(PurchaseOrderStatus::Draft);
        assert!(loaded(Some(&draft), &lines).is_some_and(|m| m.contains("not open")));
        let open = order(PurchaseOrderStatus::PartiallyReceived);
        assert_eq!(loaded(Some(&open), &lines), None);
        let stray = vec![line(1, 99, 2, 10_000, 10_000)];
        assert!(loaded(Some(&open), &stray).is_some_and(|m| m.contains("names no line")));
        let other_item = vec![line(1, 1, 5, 10_000, 10_000)];
        assert!(loaded(Some(&open), &other_item).is_some_and(|m| m.contains("not the item")));
    }

    #[test]
    fn an_empty_receipt_and_one_that_received_nothing_are_refused() {
        assert!(input_of(Vec::new(), false).lines_problem.is_some());
        let zero = line(1, 1, 2, 0, 0);
        assert!(input_of(vec![zero], false).lines_problem.is_some());
    }

    #[test]
    fn the_over_receipt_is_the_largest_over_the_order_lines() {
        // 105 of 100: exactly the tolerance, not above it.
        let at = input_of(vec![line(1, 1, 2, 105_000, 105_000)], false);
        assert_eq!(at.percentage_bps, 500);
        // 106 of 100: above it.
        let over = input_of(vec![line(1, 1, 2, 106_000, 106_000)], false);
        assert_eq!(over.percentage_bps, 600);
        // Two lines of the receipt on one order line add up.
        let two = input_of(
            vec![line(1, 1, 2, 60_000, 60_000), line(2, 1, 2, 46_000, 46_000)],
            false,
        );
        assert_eq!(two.percentage_bps, 600);
        // Rejected quantity does not count.
        let rejected = input_of(vec![line(1, 1, 2, 120_000, 100_000)], false);
        assert_eq!(rejected.percentage_bps, 0);
    }

    #[test]
    fn the_first_problem_follows_the_order_of_the_guards() {
        let input = PostInput {
            quantities_problem: Some("b".into()),
            lots_problem: Some("c".into()),
            ..PostInput::default()
        };
        assert_eq!(input.first_problem(), Some("b"));
        assert_eq!(PostInput::default().first_problem(), None);
    }

    #[test]
    fn post_is_a_scoped_user_transition_and_post_approved_a_system_one() {
        let machine = GoodsReceipt::machine();
        let post = machine.find(POST).expect("post");
        assert!(!post.is_system());
        assert_eq!(post.from_states(), ["draft"]);
        assert_eq!(post.to_state(), Some("posted"));
        assert_eq!(
            post.guard_names(),
            [
                "has_lines",
                "source_is_an_open_order",
                "quantities_balance",
                "rejections_explained",
                "lots_and_expiry"
            ]
        );
        let approved = machine.find(POST_APPROVED).expect("post_approved");
        assert!(approved.is_system());
        assert_eq!(approved.guard_names(), post.guard_names());
        assert_eq!(machine.initial_states(), ["draft"]);
    }

    #[test]
    fn a_wanted_outcome_defaults_to_replace() {
        assert_eq!(line(1, 1, 2, 1, 1).wanted_outcome, WantedOutcome::Replace);
    }
}
