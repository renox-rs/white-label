//! The stock side of a production batch (06 F&B §7.1 *Production batch*,
//! wl-inventory §3 *Lot*, §5.7 recall): the kernel's
//! [`BatchStock`](wl_kernel::inventory::BatchStock).
//!
//! - **produce**: the batch finishes. Each ingredient lot is consumed from the
//!   kitchen's location with the batch as the source (a lot on hold is
//!   consumed too: the food was physically used), the output lot is made with
//!   the expiry the chef entered, and one `produce` movement with the same
//!   source brings it in. That pairing is what the recall trace reads to put
//!   the output lot in the scope of a recalled ingredient lot
//!   ([`crate::recall_trace`]). The output cost is the cost of the ingredient
//!   lots used; the item's average cost is not changed by it.
//! - **hold_lot**: a hold on the whole lot (`LotHeld` follows the commit).
//! - **waste_lot**: what is left of a released lot posts as `wastage`
//!   movements (the batch's own discard, `STK.ACL.02` is the batch's rule). A
//!   lot on hold is wasted only through the decision *discard* of its hold
//!   line, which posts the movement itself.
//! - **wasted_of_lot**: the wastage movements already posted against a lot.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    BatchStock, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE, LotInfo, Produced, ProduceBatch, StockFuture,
    Wasted,
};

use crate::balance;
use crate::hold_service::{NewHold, NewScope, place};
use crate::item::{Item, value_of};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::stock_hold::{HoldLineStatus, HoldStatus, StockHold, StockHoldLine};
use crate::texts::said;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Why `lot` is on hold: the reason of the open hold with an undecided line of
/// it. `quality` when no such hold is found.
async fn held_reason(db: &Db, lot: &Lot) -> Result<Option<String>> {
    if lot.status != LotStatus::OnHold {
        return Ok(None);
    }
    let lines = StockHoldLine::query()
        .where_(StockHoldLine::LOT_ID.eq(Some(lot.id)))
        .where_(StockHoldLine::STATUS.eq(HoldLineStatus::Undecided))
        .order_by(StockHoldLine::ID)
        .get(db)
        .await?;
    for line in lines {
        if let Some(hold) = StockHold::find(db, line.stock_hold_id).await?
            && hold.status == HoldStatus::Open
        {
            return Ok(Some(hold.reason));
        }
    }
    Ok(Some("quality".to_owned()))
}

/// The lot as a vertical reads it.
async fn info(db: &Db, lot_id: i64) -> Result<Option<LotInfo>> {
    let Some(lot) = Lot::find(db, lot_id).await? else {
        return Ok(None);
    };
    let reason = held_reason(db, &lot).await?;
    let item = Item::find_or_404(db, lot.item_id).await?;
    let left = balance::lot_left(db, lot.id).await?;
    Ok(Some(LotInfo {
        lot: Subject::new(LOT_SUBJECT_TYPE, lot.id),
        item: Subject::new(ITEM_SUBJECT_TYPE, lot.item_id),
        number: lot.number.clone(),
        expiry_date: lot.expiry_date,
        status: lot.status.as_str().to_owned(),
        held_reason: reason,
        left_milli: left,
        value: value_of(left, item.average_cost),
    }))
}

/// Consumes the ingredient lots and makes the output lot.
async fn produce(state: &AppState, request: ProduceBatch) -> Result<Option<Produced>> {
    if request.quantity_milli <= 0 || request.inputs.is_empty() {
        return Err(refused(said("wl_inventory.batch.bad_request", &[])));
    }
    let item = Item::find_or_404(&state.db, request.item.subject_id).await?;
    let location_id = request.location.subject_id;
    let mut held = None;
    for input in &request.inputs {
        let lot = Lot::find_or_404(&state.db, input.lot.subject_id).await?;
        if lot.status == LotStatus::OnHold && held.is_none() {
            held = Some(
                held_reason(&state.db, &lot)
                    .await?
                    .unwrap_or_else(|| "quality".to_owned()),
            );
        }
    }
    let mut tx = state.db.begin().await?;
    let mut total_value = 0_i64;
    for input in &request.inputs {
        let lot = Lot::find_or_404(&mut tx, input.lot.subject_id).await?;
        let ingredient = Item::find_or_404(&mut tx, lot.item_id).await?;
        let left = balance::on_hand_at(&mut tx, ingredient.id, location_id, Some(lot.id)).await?;
        if left < input.quantity_milli || input.quantity_milli <= 0 {
            return Err(refused(said(
                "wl_inventory.batch.lot_short",
                &[
                    ("lot", &lot.number),
                    ("left", &left),
                    ("used", &input.quantity_milli),
                ],
            )));
        }
        let value = value_of(input.quantity_milli, ingredient.average_cost);
        total_value += value;
        let mut moved = StockMovement::leaving(
            MovementType::Consume,
            ingredient.id,
            Some(lot.id),
            location_id,
            input.quantity_milli,
            ingredient.average_cost,
            value,
            &request.source,
            None,
            request.by,
        );
        moved.insert(&mut tx).await?;
    }
    let mut out = Lot {
        item_id: item.id,
        number: request.lot_number.clone(),
        source_type: Some(request.source.subject_type.clone()),
        source_id: Some(request.source.subject_id),
        expiry_date: Some(request.expiry),
        status: LotStatus::Released,
        ..Lot::default()
    };
    out.insert(&mut tx).await?;
    let unit_cost = i64::try_from(i128::from(total_value) * 1_000 / i128::from(request.quantity_milli))
        .unwrap_or(0);
    let mut made = StockMovement::moving(
        MovementType::Produce,
        item.id,
        Some(out.id),
        None,
        Some(location_id),
        request.quantity_milli,
        unit_cost,
        total_value,
        &request.source,
        request.by,
    );
    made.insert(&mut tx).await?;
    tx.commit().await?;
    Ok(Some(Produced {
        lot: Subject::new(LOT_SUBJECT_TYPE, out.id),
        held_reason: held,
    }))
}

/// The wastage movements posted against `lot_id`.
async fn wasted(db: &Db, lot_id: i64) -> Result<Wasted> {
    let moves = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot_id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Wastage))
        .get(db)
        .await?;
    Ok(Wasted {
        quantity_milli: moves.iter().map(|m| m.quantity_milli).sum(),
        value: moves.iter().map(|m| m.value).sum(),
        movements: moves.len() as i64,
    })
}

/// Posts what is left of released `lot_id` as wastage.
async fn waste(state: &AppState, lot_id: i64, reason: &str, by: Option<i64>) -> Result<Wasted> {
    let lot = Lot::find_or_404(&state.db, lot_id).await?;
    if lot.status == LotStatus::OnHold {
        return Err(refused(said("wl_inventory.batch.on_hold", &[("lot", &lot.number)])));
    }
    let item = Item::find_or_404(&state.db, lot.item_id).await?;
    let source = Subject::new(LOT_SUBJECT_TYPE, lot.id);
    let mut done = Wasted::default();
    let mut tx = state.db.begin().await?;
    for (place, quantity) in balance::lot_locations(&mut tx, lot.id).await? {
        let value = value_of(quantity, item.average_cost);
        let mut moved = StockMovement::leaving(
            MovementType::Wastage,
            item.id,
            Some(lot.id),
            place,
            quantity,
            item.average_cost,
            value,
            &source,
            Some(reason.to_owned()),
            by,
        );
        moved.insert(&mut tx).await?;
        done.quantity_milli += quantity;
        done.value += value;
        done.movements += 1;
    }
    tx.commit().await?;
    Ok(done)
}

/// The system's [`BatchStock`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryBatches;

impl BatchStock for InventoryBatches {
    fn lot_info<'a>(
        &'a self,
        state: &'a AppState,
        lot: &'a Subject,
    ) -> StockFuture<'a, Option<LotInfo>> {
        Box::pin(async move {
            if lot.subject_type != LOT_SUBJECT_TYPE {
                return Ok(None);
            }
            info(&state.db, lot.subject_id).await
        })
    }

    fn produce<'a>(
        &'a self,
        state: &'a AppState,
        request: ProduceBatch,
    ) -> StockFuture<'a, Option<Produced>> {
        Box::pin(async move { produce(state, request).await })
    }

    fn hold_lot<'a>(
        &'a self,
        state: &'a AppState,
        lot: &'a Subject,
        reason: &'a str,
        reference: &'a Subject,
    ) -> StockFuture<'a, bool> {
        Box::pin(async move {
            place(
                state,
                NewHold {
                    reason: reason.to_owned(),
                    reference: reference.clone(),
                    scope: NewScope::Lots(vec![lot.subject_id]),
                    by: None,
                },
            )
            .await?;
            Ok(true)
        })
    }

    fn waste_lot<'a>(
        &'a self,
        state: &'a AppState,
        lot: &'a Subject,
        reason: &'a str,
        by: Option<i64>,
    ) -> StockFuture<'a, Wasted> {
        Box::pin(async move { waste(state, lot.subject_id, reason, by).await })
    }

    fn wasted_of_lot<'a>(
        &'a self,
        state: &'a AppState,
        lot: &'a Subject,
    ) -> StockFuture<'a, Wasted> {
        Box::pin(async move { wasted(&state.db, lot.subject_id).await })
    }
}
