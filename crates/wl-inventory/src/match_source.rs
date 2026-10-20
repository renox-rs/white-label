//! The purchase order lines for the supplier bill match (wl-inventory §6
//! `PurchaseMatchSource`, `FIN.PAB.01`): the quantity ordered and the quantity
//! accepted so far. Rejected goods are not in it, so the match sees accepted
//! quantity only.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    ITEM_SUBJECT_TYPE, MatchFuture, MatchLine, PURCHASE_ORDER_SUBJECT_TYPE, PurchaseMatchSource,
};

use crate::purchase_order::{PurchaseOrderLine, lines_of};

/// The module's [`PurchaseMatchSource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PurchaseMatch;

/// `line` as the match reads it.
pub fn match_line(line: &PurchaseOrderLine) -> MatchLine {
    MatchLine {
        po_line: line.id,
        item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
        ordered_qty_milli: line.quantity_milli,
        accepted_qty_milli: line.accepted_qty_milli,
        unit_price: line.unit_price,
    }
}

impl PurchaseMatchSource for PurchaseMatch {
    fn lines<'a>(
        &'a self,
        db: &'a Db,
        purchase_order: &'a Subject,
    ) -> MatchFuture<'a, Vec<MatchLine>> {
        Box::pin(async move {
            if purchase_order.subject_type != PURCHASE_ORDER_SUBJECT_TYPE {
                return Ok(Vec::new());
            }
            Ok(lines_of(db, purchase_order.subject_id)
                .await?
                .iter()
                .map(match_line)
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_shows_ordered_and_accepted_quantity() {
        let line = PurchaseOrderLine {
            id: 11,
            item_id: 9,
            quantity_milli: 100_000,
            accepted_qty_milli: 80_000,
            unit_price: 1_200,
            ..PurchaseOrderLine::default()
        };
        let matched = match_line(&line);
        assert_eq!(matched.po_line, 11);
        assert_eq!(matched.item.to_string(), "inventory.item:9");
        assert_eq!(
            (matched.ordered_qty_milli, matched.accepted_qty_milli),
            (100_000, 80_000)
        );
    }
}
