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
use wl_kernel::inventory::{
    GoodsReceived, GoodsReceivedLine, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    ReturnOutcome, SupplierReturnCredited, SupplierReturnDispatched, SupplierReturnDispatchedLine,
};
use wl_kernel::invoicing::{
    PostCost, SupplierDebitNoteCancelled, SupplierDebitNoteId, SupplierDebitNoteIssued, post_cost,
};

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, lines_of};
use crate::item::value_of;
use crate::purchase_order::PurchaseOrderLine;
use crate::receipt_post::{POST, POST_APPROVED};
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
