//! The kernel events wl-inventory emits (§6), from a listener on
//! [`Transitioned<GoodsReceipt>`], so only after a commit and never on a
//! rollback: `post` and `post_approved` give [`GoodsReceived`] with the
//! accepted and the rejected quantity of every line.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::inventory::{GoodsReceived, GoodsReceivedLine, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE};
use wl_kernel::Subject;

use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine, lines_of};
use crate::item::value_of;
use crate::receipt_post::{POST, POST_APPROVED};

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

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<GoodsReceipt>, state: AppState| async move {
        if ![POST, POST_APPROVED].contains(&event.transition.as_str()) {
            return Ok(());
        }
        // The committed rows.
        let receipt = GoodsReceipt::find_or_404(&state.db, event.key).await?;
        let lines = lines_of(&state.db, receipt.id).await?;
        state.emit(received_event(&receipt, &lines)).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(event.purchase_order.unwrap().to_string(), "inventory.purchase_order:3");
        assert_eq!(event.lines[0].accepted_qty_milli, 80_000);
        assert_eq!(event.lines[0].rejected_qty_milli, 20_000);
        assert_eq!(event.lines[0].reason.as_deref(), Some("temperature"));
        assert_eq!(event.lines[0].value, 96_000);
        // A reason on a line with nothing rejected is not reported.
        assert_eq!(event.lines[1].reason, None);
        assert_eq!(event.value, 96_000 + 12_000);
    }
}
