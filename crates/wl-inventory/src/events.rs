//! The kernel events wl-inventory emits (§6), from listeners on
//! [`Transitioned<GoodsReceipt>`] and [`Transitioned<SupplierReturn>`], so
//! only after a commit and never on a rollback: `post` and `post_approved`
//! give [`GoodsReceived`] with the accepted and the rejected quantity of every
//! line, and the return of the goods refused at the door that `post`
//! dispatched gives [`SupplierReturnDispatched`] after it; a user's `dispatch`
//! and `dispatch_approved` give [`SupplierReturnDispatched`] with the lines
//! (item, lot, quantity, order line, order price, wanted outcome, reason); a
//! return's `credit`, `credit_approved` and `credit_covered` give
//! [`SupplierReturnCredited`] after the wastage of a refused credit was
//! posted through the `CostPoster` (against the return as cost object, under
//! one key per return so a repeat posts nothing twice). The supplier debit
//! notes issued and cancelled by wl-invoicing are handled in
//! [`crate::return_credit`].

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::backorders::{ArrivedLine, StockArrivalReversed, StockArrived};
use wl_kernel::inventory::{
    GoodsReceived, GoodsReceivedLine, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    PurchaseOrderRevised, PurchaseOrderRevisedLine, ReturnOutcome, SupplierReturnCredited,
    SupplierReturnDispatched, SupplierReturnDispatchedLine,
};
use wl_kernel::invoicing::{
    PostCost, SupplierDebitNoteCancelled, SupplierDebitNoteId, SupplierDebitNoteIssued, post_cost,
};

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, lines_of};
use crate::item::{Item, value_of};
use crate::landed_adjust::{ADJUST, ADJUST_APPROVED};
use crate::landed_cost::{ALLOCATE, LandedCost};
use crate::po_revise::{self, PurchaseOrderRevision, RevisionLine};
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, lines_of as order_lines_of};
use crate::receipt_post::{POST, POST_APPROVED};
use crate::receipt_reverse::REVERSE;
use crate::return_credit::{CREDIT, CREDIT_APPROVED, CREDIT_COVERED};
use crate::return_dispatch::{DISPATCH, DISPATCH_APPROVED};
use crate::supplier_return::{
    ReturnStatus, SupplierReturn, SupplierReturnLine, WantedOutcome, of_source,
};
use crate::texts::said;

/// The event of the posted `receipt` with its `lines`.
pub fn received_event(receipt: &GoodsReceipt, lines: &[GoodsReceiptLine]) -> GoodsReceived {
    let lines: Vec<GoodsReceivedLine> = lines
        .iter()
        .map(|line| GoodsReceivedLine {
            po_line: line.po_line_id,
            item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
            received_qty_milli: line.received_qty_milli,
            accepted_qty_milli: line.accepted_qty_milli,
            rejected_qty_milli: line.rejected_qty_milli,
            reason: line
                .rejection_reason
                .clone()
                .filter(|_| line.rejected_qty_milli > 0),
            value: value_of(line.accepted_qty_milli, line.unit_cost),
        })
        .collect();
    GoodsReceived {
        receipt: receipt.subject(),
        purchase_order: receipt.purchase_order(),
        supplier: receipt.supplier(),
        location: Subject::new(LOCATION_SUBJECT_TYPE, receipt.location_id),
        value: lines.iter().map(|line| line.value).sum(),
        lines,
    }
}

/// The event of the dispatched `ret` with its `lines`. `prices` gives the
/// order's price of an order line; a line without one is at its unit cost.
pub fn dispatched_event(
    ret: &SupplierReturn,
    lines: &[SupplierReturnLine],
    prices: &BTreeMap<i64, i64>,
) -> SupplierReturnDispatched {
    SupplierReturnDispatched {
        supplier_return: ret.subject(),
        supplier: wl_kernel::PartyId(ret.supplier_party_id),
        source: ret.source(),
        lines: lines
            .iter()
            .map(|line| SupplierReturnDispatchedLine {
                item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
                lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
                quantity_milli: line.quantity_milli,
                po_line: line.po_line_id,
                po_price: line
                    .po_line_id
                    .and_then(|id| prices.get(&id).copied())
                    .unwrap_or(line.unit_cost),
                wanted_outcome: match line.wanted_outcome {
                    WantedOutcome::Credit => ReturnOutcome::Credit,
                    WantedOutcome::Replace => ReturnOutcome::Replace,
                },
                reason: line.reason.clone(),
            })
            .collect(),
    }
}

/// Emits [`SupplierReturnDispatched`] for return `id`, read as committed.
async fn emit_dispatched(state: &AppState, id: i64) -> Result {
    let ret = SupplierReturn::find_or_404(&state.db, id).await?;
    let lines = crate::supplier_return::lines_of(&state.db, ret.id).await?;
    let mut prices = BTreeMap::new();
    for line in &lines {
        if let Some(po_line_id) = line.po_line_id
            && let Some(po_line) = PurchaseOrderLine::find(&state.db, po_line_id).await?
        {
            prices.insert(po_line.id, po_line.unit_price);
        }
    }
    // The difference between the receipt's cost and the average cost goes to
    // the price variance, once per line (the key makes a repeat harmless).
    for line in lines.iter().filter(|line| line.variance != 0) {
        let request = PostCost::new(
            ret.subject(),
            format!("inventory.supplier_return_variance:{}", line.id),
            ret.subject(),
            line.variance,
        )
        .cost_code("price_variance");
        post_cost(state, request).await?;
    }
    state.emit(dispatched_event(&ret, &lines, &prices)).await
}

/// Posts the wastage of credited return `id` and emits
/// [`SupplierReturnCredited`], read as committed.
async fn emit_credited(state: &AppState, id: i64) -> Result {
    let ret = SupplierReturn::find_or_404(&state.db, id).await?;
    if ret.wastage_fils > 0 {
        let request = PostCost::new(
            ret.subject(),
            format!("inventory.supplier_return_wastage:{}", ret.id),
            ret.subject(),
            ret.wastage_fils,
        )
        .cost_code("wastage");
        post_cost(state, request).await?;
    }
    // The task to chase the supplier is done with the return.
    let creator = wl_kernel::tickets::task_creator(state);
    if let Some(task) = creator
        .find_open(state, &crate::chase::task_key(ret.id))
        .await?
    {
        let note = said("wl_inventory.chase.resolved", &[]);
        creator.resolve_from_source(state, task.id, &note).await?;
    }
    state
        .emit(SupplierReturnCredited {
            supplier_return: ret.subject(),
            supplier: wl_kernel::PartyId(ret.supplier_party_id),
            basis: ret.credit_basis.clone().unwrap_or_default(),
            debit_note: ret.debit_note_id.map(SupplierDebitNoteId),
            reason: ret.credit_reason.clone(),
            wastage: ret.wastage_fils,
        })
        .await
}

/// The event of order `order` now at revision `revision` after `earlier`,
/// the revision it replaced, with the lines whose quantity or price changed.
pub fn revised_event(
    order: &PurchaseOrder,
    earlier: &PurchaseOrderRevision,
    lines: &[PurchaseOrderLine],
) -> PurchaseOrderRevised {
    let before: BTreeMap<i64, RevisionLine> = earlier
        .lines()
        .into_iter()
        .map(|line| (line.po_line_id, line))
        .collect();
    let changed = lines
        .iter()
        .filter_map(|line| {
            let old = before.get(&line.id)?;
            if old.quantity_milli == line.quantity_milli && old.unit_price == line.unit_price {
                return None;
            }
            Some(PurchaseOrderRevisedLine {
                po_line: line.id,
                item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
                old_quantity_milli: old.quantity_milli,
                new_quantity_milli: line.quantity_milli,
                old_unit_price: old.unit_price,
                new_unit_price: line.unit_price,
            })
        })
        .collect();
    PurchaseOrderRevised {
        purchase_order: order.subject(),
        supplier: order.supplier(),
        revision: order.revision,
        currency: order.currency.clone(),
        lines: changed,
        old_total: earlier.total,
        new_total: crate::purchase_order::total_of(lines),
        reason: earlier.reason.clone(),
    }
}

/// Emits [`PurchaseOrderRevised`] for order `id` and sends the supplier the
/// revised document, read as committed.
async fn emit_revised(state: &AppState, id: i64) -> Result {
    let order = PurchaseOrder::find_or_404(&state.db, id).await?;
    let Some(earlier) = PurchaseOrderRevision::query()
        .where_(PurchaseOrderRevision::PURCHASE_ORDER_ID.eq(order.id))
        .where_(PurchaseOrderRevision::REVISION.eq(order.revision - 1))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    let lines = order_lines_of(&state.db, order.id).await?;
    let event = revised_event(&order, &earlier, &lines);
    let number = order.number.clone().unwrap_or_default();
    let request = wl_kernel::notify::NotifyRequest::new(po_revise::NOTICE, order.subject())
        .subject_label(&number)
        .to_party(order.supplier())
        .data(renox::serde_json::json!({
            "number": number,
            "revision": order.revision.to_string(),
            "total": po_revise::money_text(event.new_total),
            "currency": order.currency,
            "reason": earlier.reason,
            "link": format!(
                "/purchase-orders/{}/revisions/{}/document",
                order.id, order.revision
            ),
        }));
    state.emit(event).await?;
    wl_kernel::notify::notifier(state)
        .notify(state, request)
        .await?;
    Ok(())
}

/// The arrival of `quantities` (item id and thousandths, summed per item),
/// with the item codes. Empty when nothing arrived.
async fn arrived_lines(
    state: &AppState,
    quantities: impl IntoIterator<Item = (i64, i64)>,
) -> Result<Vec<ArrivedLine>> {
    let mut per_item: BTreeMap<i64, i64> = BTreeMap::new();
    for (item_id, quantity_milli) in quantities {
        if quantity_milli > 0 {
            *per_item.entry(item_id).or_insert(0) += quantity_milli;
        }
    }
    let mut lines = Vec::new();
    for (item_id, quantity_milli) in per_item {
        if let Some(item) = Item::find(&state.db, item_id).await? {
            lines.push(ArrivedLine {
                item: item.subject(),
                item_code: item.code,
                quantity_milli,
            });
        }
    }
    Ok(lines)
}

/// Emits [`StockArrived`] for stock that entered `location_id` through
/// `source` (`kind`: `receive` or `transfer_in`), read as committed: wl-sales
/// reserves it for the oldest back-orders.
async fn emit_arrived(
    state: &AppState,
    source: Subject,
    kind: &str,
    location_id: i64,
    quantities: impl IntoIterator<Item = (i64, i64)>,
) -> Result {
    let lines = arrived_lines(state, quantities).await?;
    if lines.is_empty() {
        return Ok(());
    }
    state
        .emit(StockArrived {
            source,
            kind: kind.to_owned(),
            location: Subject::new(LOCATION_SUBJECT_TYPE, location_id),
            lines,
        })
        .await
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<GoodsReceipt>, state: AppState| async move {
            if ![POST, POST_APPROVED].contains(&event.transition.as_str()) {
                return Ok(());
            }
            // The committed rows.
            let receipt = GoodsReceipt::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, receipt.id).await?;
            state.emit(received_event(&receipt, &lines)).await?;
            // What was accepted joins an open hold on the location.
            crate::arrival::on_receipt(&state, &receipt, &lines).await?;
            // The oldest back-orders of the location are served from it.
            emit_arrived(
                &state,
                receipt.subject(),
                "receive",
                receipt.location_id,
                lines
                    .iter()
                    .map(|line| (line.item_id, line.accepted_qty_milli)),
            )
            .await?;
            // The goods refused at the door left with the driver as the receipt
            // posted: the system dispatched their return.
            for ret in of_source(&state.db, &receipt.subject()).await? {
                if ret.status == ReturnStatus::Dispatched {
                    emit_dispatched(&state, ret.id).await?;
                }
            }
            // A replacement that covered the last line of its return
            // credited it.
            if let Some(return_id) = receipt.supplier_return_id {
                let ret = SupplierReturn::find_or_404(&state.db, return_id).await?;
                if ret.status == ReturnStatus::Credited
                    && ret.credit_basis.as_deref() == Some("replacement")
                {
                    emit_credited(&state, ret.id).await?;
                }
            }
            Ok(())
        },
    );
    app.listen(
        |event: Transitioned<GoodsReceipt>, state: AppState| async move {
            if event.transition != REVERSE {
                return Ok(());
            }
            // The stock left again: the reservations made from it are given
            // back (BL-SAL-17).
            let receipt = GoodsReceipt::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, receipt.id).await?;
            let lines = arrived_lines(
                &state,
                lines
                    .iter()
                    .map(|line| (line.item_id, line.accepted_qty_milli)),
            )
            .await?;
            if lines.is_empty() {
                return Ok(());
            }
            state
                .emit(StockArrivalReversed {
                    source: receipt.subject(),
                    location: Subject::new(LOCATION_SUBJECT_TYPE, receipt.location_id),
                    lines,
                })
                .await
        },
    );
    app.listen(
        |event: Transitioned<crate::lot::Lot>, state: AppState| async move {
            if event.transition != crate::lot::HOLD {
                return Ok(());
            }
            let lot = crate::lot::Lot::find_or_404(&state.db, event.key).await?;
            state
                .emit(wl_kernel::inventory::LotHeld {
                    lot: Subject::new(LOT_SUBJECT_TYPE, lot.id),
                    item: Subject::new(ITEM_SUBJECT_TYPE, lot.item_id),
                    reason: event.reason.clone().unwrap_or_default(),
                    quantity_milli: crate::balance::lot_left(&state.db, lot.id).await?,
                })
                .await
        },
    );
    app.listen(
        |event: Transitioned<crate::transfer::Transfer>, state: AppState| async move {
            if event.transition != crate::transfer::RECEIVE {
                return Ok(());
            }
            let transfer = crate::transfer::Transfer::find_or_404(&state.db, event.key).await?;
            crate::arrival::on_transfer(&state, &transfer).await?;
            let lines = crate::transfer::lines_of(&state.db, transfer.id).await?;
            emit_arrived(
                &state,
                transfer.subject(),
                "transfer_in",
                transfer.to_location_id,
                lines.iter().map(|line| (line.item_id, line.quantity_milli)),
            )
            .await
        },
    );
    app.listen(
        |event: Transitioned<crate::stock_hold::StockHold>, state: AppState| async move {
            use crate::stock_hold::{ADD_LINES, ADD_LINES_BY_SYSTEM};
            if ![ADD_LINES, ADD_LINES_BY_SYSTEM].contains(&event.transition.as_str()) {
                return Ok(());
            }
            let added = event
                .meta
                .get("added")
                .and_then(renox::serde_json::Value::as_u64)
                .unwrap_or(0);
            crate::hold_service::announce_added(&state, event.key, added as usize).await
        },
    );
    app.listen(
        |event: Transitioned<LandedCost>, state: AppState| async move {
            let name = event.transition.as_str();
            if [ALLOCATE, ADJUST, ADJUST_APPROVED].contains(&name) {
                crate::landed_adjust::announce(&state, event.key).await
            } else {
                Ok(())
            }
        },
    );
    app.listen(
        |event: Transitioned<PurchaseOrder>, state: AppState| async move {
            if po_revise::TRANSITIONS.contains(&event.transition.as_str()) {
                emit_revised(&state, event.key).await
            } else {
                Ok(())
            }
        },
    );
    app.listen(
        |event: Transitioned<SupplierReturn>, state: AppState| async move {
            let name = event.transition.as_str();
            if [DISPATCH, DISPATCH_APPROVED].contains(&name) {
                emit_dispatched(&state, event.key).await
            } else if [CREDIT, CREDIT_APPROVED, CREDIT_COVERED].contains(&name) {
                emit_credited(&state, event.key).await
            } else {
                Ok(())
            }
        },
    );
    app.listen(
        |event: wl_kernel::invoicing::SupplierBillApproved, state: AppState| async move {
            crate::landed_draft::on_bill_approved(&state, &event).await
        },
    );
    app.listen(
        |event: SupplierDebitNoteIssued, state: AppState| async move {
            crate::return_credit::on_debit_note_issued(&state, &event).await
        },
    );
    app.listen(
        |event: SupplierDebitNoteCancelled, state: AppState| async move {
            crate::return_credit::on_debit_note_cancelled(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dispatched_event_names_the_order_price_and_the_outcome() {
        let ret = SupplierReturn {
            id: 4,
            supplier_party_id: 7,
            source_type: "inventory.goods_receipt".into(),
            source_id: 5,
            ..SupplierReturn::default()
        };
        let line = SupplierReturnLine {
            item_id: 9,
            lot_id: Some(2),
            quantity_milli: 10_000,
            unit_cost: 4_800,
            reason: "damaged".into(),
            wanted_outcome: WantedOutcome::Credit,
            po_line_id: Some(11),
            ..SupplierReturnLine::default()
        };
        let other = SupplierReturnLine {
            po_line_id: None,
            lot_id: None,
            wanted_outcome: WantedOutcome::Replace,
            ..line.clone()
        };
        let event = dispatched_event(&ret, &[line, other], &BTreeMap::from([(11, 5_000)]));
        assert_eq!(
            event.supplier_return.to_string(),
            "inventory.supplier_return:4"
        );
        assert_eq!(event.source.to_string(), "inventory.goods_receipt:5");
        assert_eq!(event.lines[0].po_price, 5_000);
        assert_eq!(event.lines[0].wanted_outcome, ReturnOutcome::Credit);
        assert_eq!(
            event.lines[0].lot.as_ref().unwrap().to_string(),
            "inventory.lot:2"
        );
        // No order line: the line's own cost.
        assert_eq!(event.lines[1].po_price, 4_800);
        assert_eq!(event.lines[1].wanted_outcome, ReturnOutcome::Replace);
        assert_eq!(event.lines[1].lot, None);
    }

    #[test]
    fn the_event_shows_accepted_and_rejected_quantity_per_line() {
        let receipt = GoodsReceipt {
            id: 5,
            purchase_order_id: Some(3),
            supplier_party_id: 7,
            location_id: 2,
            ..GoodsReceipt::default()
        };
        let line = GoodsReceiptLine {
            po_line_id: Some(11),
            item_id: 9,
            received_qty_milli: 100_000,
            accepted_qty_milli: 80_000,
            rejected_qty_milli: 20_000,
            rejection_reason: Some("temperature".into()),
            unit_cost: 1_200,
            ..GoodsReceiptLine::default()
        };
        let clean = GoodsReceiptLine {
            rejection_reason: Some("stale".into()),
            rejected_qty_milli: 0,
            accepted_qty_milli: 10_000,
            received_qty_milli: 10_000,
            ..line.clone()
        };
        let event = received_event(&receipt, &[line, clean]);
        assert_eq!(event.receipt.to_string(), "inventory.goods_receipt:5");
        assert_eq!(
            event.purchase_order.unwrap().to_string(),
            "inventory.purchase_order:3"
        );
        assert_eq!(event.lines[0].accepted_qty_milli, 80_000);
        assert_eq!(event.lines[0].rejected_qty_milli, 20_000);
        assert_eq!(event.lines[0].reason.as_deref(), Some("temperature"));
        assert_eq!(event.lines[0].value, 96_000);
        // A reason on a line with nothing rejected is not reported.
        assert_eq!(event.lines[1].reason, None);
        assert_eq!(event.value, 96_000 + 12_000);
    }
}
