//! Goods a customer brings back that are under a recall (wl-inventory §4
//! *Recall*, the paragraph after the trace table; wl-sales §4.5 `receive`,
//! BL-SAL-15): the kernel's [`LotTrace::recall_of`] says whether returned
//! goods are in an open recall, and [`LotTrace::receive_returned`] takes them
//! into quarantine and onto the recall's stock hold. The recall, not the
//! return, decides what happens to them.
//!
//! - Goods of a lot are in a recall when the lot is in the scope of a recall
//!   that is `open` or `traced`.
//! - Goods of an item not tracked by lot are in a recall when the item is in
//!   its scope by received dates and the day they were sold falls from the
//!   first receipt in the range to the trace end date (today when none).
//! - Only a recall whose stock hold is still `open` takes goods back: once
//!   every line of the hold is decided there is nothing left to join, and the
//!   return is an ordinary one.
//! - Receiving writes a `return_in` movement to the quarantine location (the
//!   source is the return) and one line on the hold for that quantity, in one
//!   transaction. The hold already blocks the lot wherever it is, and a line
//!   that names no lot blocks the quantity at the quarantine location.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::{RECALL_SUBJECT_TYPE, ReturnedGoods};

use crate::item::{Item, value_of};
use crate::location::quarantine_location;
use crate::movement::{MovementType, StockMovement};
use crate::recall::{Recall, RecallLot, RecallScope, RecallStatus};
use crate::stock_hold::{HoldStatus, StockHold, new_line};
use crate::texts::said;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The recall `goods` are in, if any.
///
/// # Errors
///
/// A database error.
pub async fn recall_of(db: &Db, goods: &ReturnedGoods) -> Result<Option<Recall>> {
    let Some(item) = Item::query()
        .where_(Item::CODE.eq(goods.item_code.as_str()))
        .first(db)
        .await?
    else {
        return Ok(None);
    };
    let recalls = Recall::query().order_by(Recall::ID).get(db).await?;
    for recall in recalls {
        if recall.status == RecallStatus::Closed {
            continue;
        }
        let Some(hold_id) = recall.stock_hold_id else {
            continue;
        };
        let hold = StockHold::find_or_404(db, hold_id).await?;
        if hold.status != HoldStatus::Open {
            continue;
        }
        if let Some(lot) = &goods.lot {
            let in_scope = RecallLot::query()
                .where_(RecallLot::RECALL_ID.eq(recall.id))
                .where_(RecallLot::LOT_ID.eq(lot.subject_id))
                .exists(db)
                .await?;
            if in_scope {
                return Ok(Some(recall));
            }
        }
        if !item.lot_tracked
            && let Some(sold_on) = goods.sold_on
        {
            let scopes = RecallScope::query()
                .where_(RecallScope::RECALL_ID.eq(recall.id))
                .where_(RecallScope::ITEM_ID.eq(item.id))
                .get(db)
                .await?;
            let inside = scopes.iter().any(|scope| {
                scope.first_receipt_on.is_some_and(|first| {
                    first <= sold_on && sold_on <= scope.trace_end.unwrap_or_else(today)
                })
            });
            if inside {
                return Ok(Some(recall));
            }
        }
    }
    Ok(None)
}

/// Takes `goods` into quarantine and onto the hold of recall `recall_id`.
///
/// # Errors
///
/// 422 for an item the stock does not know, a quantity that is not above zero
/// or a recall whose hold is not open; a database error.
pub async fn receive_returned(db: &Db, recall_id: i64, goods: &ReturnedGoods) -> Result<bool> {
    if goods.quantity_milli <= 0 {
        return Err(refused(said("wl_inventory.issue.bad_quantity", &[])));
    }
    let recall = Recall::find_or_404(db, recall_id).await?;
    let Some(item) = Item::query()
        .where_(Item::CODE.eq(goods.item_code.as_str()))
        .first(db)
        .await?
    else {
        return Err(refused(said(
            "wl_inventory.recall.return_item",
            &[("item", &goods.item_code)],
        )));
    };
    let hold = match recall.stock_hold_id {
        Some(id) => Some(StockHold::find_or_404(db, id).await?),
        None => None,
    }
    .filter(|hold| hold.status == HoldStatus::Open && recall.status != RecallStatus::Closed)
    .ok_or_else(|| refused(said("wl_inventory.recall.return_hold", &[])))?;
    let lot_id = goods.lot.as_ref().map(|lot| lot.subject_id);
    let mut tx = db.begin().await?;
    let quarantine = quarantine_location(&mut tx).await?;
    let mut moved = StockMovement::moving(
        MovementType::ReturnIn,
        item.id,
        lot_id,
        None,
        Some(quarantine),
        goods.quantity_milli,
        item.average_cost,
        value_of(goods.quantity_milli, item.average_cost),
        &goods.source,
        None,
    );
    moved.reason = Some("recalled goods".to_owned());
    moved.insert(&mut tx).await?;
    new_line(
        &mut tx,
        hold.id,
        item.id,
        quarantine,
        lot_id,
        goods.quantity_milli,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// The subject of `recall`.
pub fn subject_of(recall: &Recall) -> Subject {
    Subject::new(RECALL_SUBJECT_TYPE, recall.id)
}
