//! Reversing a posted goods receipt (wl-inventory §4 *Goods receipt*
//! `reverse`, issue #221). A receipt entered by mistake is taken back out:
//! `reverse` (a holder of `goods_receipts.reverse` in the receipt's scope)
//! moves it from `posted` to `reversed`.
//!
//! - the guards: no Return to supplier made from the receipt has left
//!   (`dispatched` or `credited`); the purchase order is not `closed` (a
//!   closed order was billed); and the stock is still there, all or
//!   nothing: the full accepted quantity of every line (and lot) is still at
//!   the receipt's location and the lot is not on hold, otherwise the refusal
//!   names the line and the missing quantity and nothing is posted;
//! - the effect, in the transaction: an `adjust` movement out of the
//!   location per accepted line (the reversing movement), the item's average
//!   cost without the receipt, the order lines' accepted quantity back down
//!   by the accepted quantity only (the outstanding quantity goes up), and
//!   every `draft` return from the receipt cancelled with the reason
//!   *receipt reversed*.
//!
//! Decisions: the guards read what [`gather`] found in a transaction of its
//! own that is rolled back, and the effect finds it again in the run's
//! transaction and refuses with a 422 if stock moved in between. "Not yet
//! billed" is read from the order: wl-invoicing does not tell here which
//! quantities it billed, a `closed` order is the billed one. "Reserved" has
//! no data yet (reservations come with the issues that build them); an
//! `on_hold` lot is the held stock. A reversal that empties an order line
//! leaves the order in the state it was in, except that a `received` order
//! goes back to `partially_received` (its `reopen`).

use std::collections::BTreeMap;

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use crate::item::{Item, average_after_removal, quantity_text, value_of};
use crate::lot::{Lot, LotStatus};
use crate::movement::{self, MovementType, StockMovement};
use crate::permissions;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use crate::supplier_return::{self, ReturnStatus, SupplierReturn};
use crate::texts::said;

/// The `reverse` transition's name.
pub const REVERSE: &str = "reverse";

/// What a `reverse` run carries to its guards and effect, gathered by
/// [`gather`]. Each `*_problem` is the text of the first thing wrong of that
/// kind, `None` when nothing is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReverseInput {
    /// The user who reverses it.
    pub by: Option<i64>,
    /// A return from the receipt has left with the supplier.
    pub returns_problem: Option<String>,
    /// The goods were billed.
    pub billed_problem: Option<String>,
    /// Part of the accepted quantity is no longer there.
    pub stock_problem: Option<String>,
}

impl ReverseInput {
    /// The first problem, in the order the guards run.
    pub fn first_problem(&self) -> Option<&str> {
        [&self.returns_problem, &self.billed_problem, &self.stock_problem]
            .into_iter()
            .find_map(|problem| problem.as_deref())
    }
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The lines of `receipt`, read in `tx`.
async fn lines_in(tx: &mut Transaction, receipt_id: i64) -> Result<Vec<GoodsReceiptLine>> {
    GoodsReceiptLine::query()
        .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(receipt_id))
        .order_by(GoodsReceiptLine::POSITION)
        .get(&mut *tx)
        .await
}

/// What stands in the way of reversing `receipt`, read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    receipt: &GoodsReceipt,
    by: Option<i64>,
) -> Result<ReverseInput> {
    let mut input = ReverseInput {
        by,
        ..ReverseInput::default()
    };
    let returns = supplier_return::of_source_in(tx, &receipt.subject()).await?;
    if let Some(left) = returns
        .iter()
        .find(|r| matches!(r.status, ReturnStatus::Dispatched | ReturnStatus::Credited))
    {
        input.returns_problem = Some(said(
            "wl_inventory.reverse.return_left",
            &[("number", left.number.as_deref().unwrap_or("-"))],
        ));
    }
    if let Some(order_id) = receipt.purchase_order_id {
        let order = PurchaseOrder::find(&mut *tx, order_id).await?;
        if order.is_some_and(|o| o.status == PurchaseOrderStatus::Closed) {
            input.billed_problem = Some(said("wl_inventory.reverse.billed", &[]));
        }
    }
    let lines = lines_in(tx, receipt.id).await?;
    input.stock_problem = stock_problem(tx, receipt, &lines).await?;
    Ok(input)
}

/// The first line whose accepted quantity is not all still at the location.
/// Lines of one item and lot add up before they are compared with what is
/// there.
async fn stock_problem(
    tx: &mut Transaction,
    receipt: &GoodsReceipt,
    lines: &[GoodsReceiptLine],
) -> Result<Option<String>> {
    let mut claimed: BTreeMap<(i64, Option<i64>), i64> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.accepted_qty_milli > 0) {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let position = line.position.to_string();
        if let Some(lot_id) = line.lot_id {
            if let Some(lot) = Lot::find(&mut *tx, lot_id).await? {
                if lot.status == LotStatus::OnHold {
                    return Ok(Some(said(
                        "wl_inventory.reverse.held",
                        &[
                            ("position", &position),
                            ("lot", &lot.number),
                            ("item", &item.code),
                            ("status", lot.status.as_str()),
                        ],
                    )));
                }
            }
        }
        let needed = claimed.entry((line.item_id, line.lot_id)).or_insert(0);
        *needed += line.accepted_qty_milli;
        let needed = *needed;
        let available = match line.lot_id {
            Some(lot_id) => {
                movement::on_hand_lot_in(tx, line.item_id, lot_id, receipt.location_id).await?
            }
            None => movement::on_hand_in(tx, line.item_id, receipt.location_id).await?,
        };
        if available < needed {
            let available = available.max(0);
            return Ok(Some(said(
                "wl_inventory.reverse.missing",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("available", &quantity_text(available)),
                    ("accepted", &quantity_text(needed)),
                    ("missing", &quantity_text(needed - available)),
                ],
            )));
        }
    }
    Ok(None)
}

fn input(ctx: &TransitionCtx) -> Option<ReverseInput> {
    ctx.input_as::<ReverseInput>().ok()
}

/// A guard that refuses with the problem `pick` finds in the run's input.
fn with_problem(
    transition: Transition<GoodsReceipt>,
    name: &'static str,
    pick: fn(&ReverseInput) -> Option<String>,
) -> Transition<GoodsReceipt> {
    transition.guard(name, move |_receipt: &GoodsReceipt, ctx| {
        let problem = input(ctx).map_or_else(
            || Some(said("wl_inventory.reverse.not_posted", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// `reverse`, for the machine of the goods receipt.
pub(crate) fn transition() -> Transition<GoodsReceipt> {
    let transition = Transition::new(REVERSE)
        .label("Reverse")
        .tone("danger")
        .permission_in(permissions::GOODS_RECEIPTS_REVERSE, |receipt: &GoodsReceipt| {
            receipt.scope()
        })
        .from([GoodsReceiptStatus::Posted])
        .to(GoodsReceiptStatus::Reversed);
    let transition = with_problem(transition, "no_return_left", |i| i.returns_problem.clone());
    let transition = with_problem(transition, "not_billed", |i| i.billed_problem.clone());
    let transition = with_problem(transition, "stock_still_available", |i| i.stock_problem.clone());
    transition.effect(|receipt: &GoodsReceipt, ctx, tx| {
        let receipt = receipt.clone();
        let by = input(ctx).and_then(|input| input.by);
        Box::pin(async move { reverse_in(tx, receipt, by).await })
    })
}

/// The effect of `reverse`, in the run's transaction.
async fn reverse_in(tx: &mut Transaction, receipt: GoodsReceipt, by: Option<i64>) -> Result {
    // Stock may have moved since the guards looked: all or nothing.
    let found = gather(tx, &receipt, by).await?;
    if let Some(problem) = found.first_problem() {
        return Err(refused(problem));
    }
    let now = renox::db::now();
    let source = receipt.subject();
    let reason = said("wl_inventory.reverse.reason", &[]);
    let lines = lines_in(tx, receipt.id).await?;
    for line in lines.iter().filter(|line| line.accepted_qty_milli > 0) {
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        let on_hand = movement::on_hand_everywhere_in(tx, item.id).await?;
        let value = value_of(line.accepted_qty_milli, line.unit_cost);
        let mut moved = StockMovement::leaving(
            MovementType::Adjust,
            item.id,
            line.lot_id,
            receipt.location_id,
            line.accepted_qty_milli,
            line.unit_cost,
            value,
            &source,
            Some(reason.clone()),
            by,
        );
        moved.insert(&mut *tx).await?;
        let average = average_after_removal(
            on_hand,
            item.average_cost,
            line.accepted_qty_milli,
            line.unit_cost,
        );
        Item::query()
            .where_(Item::ID.eq(item.id))
            .update(&mut *tx, &[("average_cost", &average), ("updated_at", &now)])
            .await?;
        // The order's outstanding quantity goes back up by the accepted
        // quantity only.
        if let Some(po_line_id) = line.po_line_id {
            let po_line = PurchaseOrderLine::find_or_404(&mut *tx, po_line_id).await?;
            let accepted = (po_line.accepted_qty_milli - line.accepted_qty_milli).max(0);
            PurchaseOrderLine::query()
                .where_(PurchaseOrderLine::ID.eq(po_line.id))
                .update(
                    &mut *tx,
                    &[("accepted_qty_milli", &accepted), ("updated_at", &now)],
                )
                .await?;
        }
    }
    // The goods that were kept for collection are still there, but the
    // return that waited for them is withdrawn with the receipt.
    let cancelled = said("wl_inventory.reverse.return_cancelled", &[]);
    for ret in supplier_return::of_source_in(tx, &source).await? {
        if ret.status != ReturnStatus::Draft {
            continue;
        }
        let _outcome = ret
            .transition(supplier_return::CANCEL)
            .by_system()
            .reason(cancelled.clone())
            .run_in(&mut *tx)
            .await
            .map_err(TransitionError::into_error)?;
    }
    Ok(())
}

/// `user` reverses receipt `receipt_id`. Refused with the first problem
/// before anything is written (see the module docs).
///
/// # Errors
///
/// `Forbidden` without `goods_receipts.reverse` in the receipt's scope;
/// `NotFrom` unless `posted`; `Guard` (422) for a problem of the receipt.
pub async fn reverse(
    state: &AppState,
    user: &AuthUser,
    receipt_id: i64,
) -> std::result::Result<GoodsReceipt, TransitionError> {
    let receipt = GoodsReceipt::find_or_404(&state.db, receipt_id).await?;
    if receipt.status != GoodsReceiptStatus::Posted {
        return Err(TransitionError::Guard(said("wl_inventory.reverse.not_posted", &[])));
    }
    // Reads in a transaction of its own, rolled back: nothing is held while
    // the transition opens its own.
    let mut tx = state.db.begin().await?;
    let found = gather(&mut tx, &receipt, Some(user.id)).await?;
    tx.rollback().await?;
    if let Some(problem) = found.first_problem() {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    let _outcome = receipt
        .transition(REVERSE)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    let reversed = GoodsReceipt::find_or_404(&state.db, receipt.id).await?;
    Ok(reversed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_problem_is_a_return_that_left_then_billing_then_stock() {
        let mut input = ReverseInput {
            stock_problem: Some("stock".into()),
            ..ReverseInput::default()
        };
        assert_eq!(input.first_problem(), Some("stock"));
        input.billed_problem = Some("billed".into());
        assert_eq!(input.first_problem(), Some("billed"));
        input.returns_problem = Some("left".into());
        assert_eq!(input.first_problem(), Some("left"));
        assert_eq!(ReverseInput::default().first_problem(), None);
    }

    #[test]
    fn reverse_is_a_user_transition_from_posted_to_reversed() {
        let machine = GoodsReceipt::machine();
        let transition = machine.find(REVERSE).expect("transition");
        assert!(!transition.is_system());
        assert_eq!(transition.from_states(), ["posted"]);
        assert_eq!(transition.to_state(), Some("reversed"));
    }
}
