//! The stock movements of a work order (07 Manufacturing §7.1 *Work order*),
//! posted inside the transaction of the caller through the kernel's
//! [`ProductionStock`]:
//!
//! - `return_material`: a `return_in` movement of unused material back to the
//!   store, with its lot, at the cost it was issued at;
//! - `waste_material`: a `wastage` movement of material lost with the work
//!   order. The issued material left the store when it was issued, so the
//!   movement has no location: it writes the value off against the work
//!   order (the source) without touching a balance;
//! - `produce`: a `produce` movement of good output into the work order's
//!   output lot at the finished goods location. The first report opens the
//!   lot in `quarantine` (the finished lot cannot be picked or issued before
//!   QC releases it, `MFG.ACL.02`) when the item is lot-tracked; an item that
//!   is not lot-tracked has no lot.
//!
//! - `output_available`, `void_output` and `void_waste` serve the void of a
//!   production report (BL-MFG-04): the good output leaves the lot again with
//!   an `adjust` movement (as a reversed goods receipt does), refused with
//!   "output already used" when the lot no longer holds the quantity at the
//!   location (moved, issued or picked); the `wastage` of the scrap is
//!   reversed with an `adjust` that has no location, as the wastage had none.
//!
//! Decisions where the spec is silent: the output lot is numbered by the
//! caller (`F-WO-2026-0001`); reservations of output lots for order lines
//! are not kept by this module yet, so `release_reservation` answers `false`
//! (the dispatch pick reserves, not the work order).

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::LOT_SUBJECT_TYPE;
use wl_kernel::production::{
    ProduceOutput, ProductionFuture, ProductionStock, ReturnMaterial, VoidOutput, WasteMaterial,
};

use crate::balance;
use crate::item::{Item, quantity_text, value_of};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Posts the return of unused material (see the module).
///
/// # Errors
///
/// 422 for a quantity that is not above zero; a database error.
pub async fn return_material(tx: &mut Transaction, request: ReturnMaterial) -> Result {
    if request.quantity_milli <= 0 {
        return Err(refused("The quantity must be above zero."));
    }
    StockMovement::moving(
        MovementType::ReturnIn,
        request.item.subject_id,
        request.lot.as_ref().map(|lot| lot.subject_id),
        None,
        Some(request.location.subject_id),
        request.quantity_milli,
        request.unit_cost,
        value_of(request.quantity_milli, request.unit_cost),
        &request.source,
        request.user,
    )
    .insert(&mut *tx)
    .await?;
    Ok(())
}

/// Posts a `wastage` of material (see the module).
///
/// # Errors
///
/// 422 for a quantity that is not above zero; a database error.
pub async fn waste_material(tx: &mut Transaction, request: WasteMaterial) -> Result {
    if request.quantity_milli <= 0 {
        return Err(refused("The quantity must be above zero."));
    }
    let unit_cost = i64::try_from(
        (i128::from(request.value) * 1_000 + i128::from(request.quantity_milli) / 2)
            / i128::from(request.quantity_milli),
    )
    .unwrap_or(0);
    let mut movement = StockMovement::moving(
        MovementType::Wastage,
        request.item.subject_id,
        request.lot.as_ref().map(|lot| lot.subject_id),
        None,
        None,
        request.quantity_milli,
        unit_cost,
        request.value,
        &request.source,
        request.user,
    );
    movement.reason = Some(request.reason);
    movement.insert(&mut *tx).await?;
    Ok(())
}

/// Posts good output into the output lot (see the module). Returns the lot.
///
/// # Errors
///
/// 422 for a quantity that is not above zero; a database error.
pub async fn produce(tx: &mut Transaction, request: ProduceOutput) -> Result<Option<Subject>> {
    if request.quantity_milli <= 0 {
        return Err(refused("The quantity must be above zero."));
    }
    let item = Item::find_or_404(&mut *tx, request.item.subject_id).await?;
    let lot_id = if item.lot_tracked {
        let id = match &request.lot {
            Some(lot) => lot.subject_id,
            None => {
                let mut lot = Lot {
                    item_id: item.id,
                    number: request.lot_number.clone(),
                    source_type: Some(request.source.subject_type.clone()),
                    source_id: Some(request.source.subject_id),
                    status: LotStatus::Quarantine,
                    ..Lot::default()
                };
                lot.insert(&mut *tx).await?;
                lot.id
            }
        };
        Some(id)
    } else {
        None
    };
    StockMovement::moving(
        MovementType::Produce,
        item.id,
        lot_id,
        None,
        Some(request.location.subject_id),
        request.quantity_milli,
        request.unit_cost,
        value_of(request.quantity_milli, request.unit_cost),
        &request.source,
        request.user,
    )
    .insert(&mut *tx)
    .await?;
    Ok(lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)))
}

/// What of the output lot (of the item when it has none) is at `location`
/// and not reserved, in thousandths.
///
/// # Errors
///
/// A database error.
pub async fn output_available<'c>(
    db: impl renox::db::Executor<'c>,
    item: &Subject,
    lot: Option<&Subject>,
    location: &Subject,
) -> Result<i64> {
    let found = balance::balance(
        db,
        item.subject_id,
        location.subject_id,
        lot.map(|lot| lot.subject_id),
    )
    .await?;
    Ok((found.on_hand_milli - found.reserved_milli).max(0))
}

/// Takes the good output of a voided report out of the lot (see the module).
///
/// # Errors
///
/// 422 for a quantity that is not above zero, or "output already used" when
/// the lot holds less than the quantity; a database error.
pub async fn void_output(tx: &mut Transaction, request: VoidOutput) -> Result {
    if request.quantity_milli <= 0 {
        return Err(refused("The quantity must be above zero."));
    }
    let left = output_available(
        &mut *tx,
        &request.item,
        request.lot.as_ref(),
        &request.location,
    )
    .await?;
    if left < request.quantity_milli {
        return Err(refused(format!(
            "Output already used: the lot holds {} of the {} the report made; the production manager corrects it with a reversing movement.",
            quantity_text(left),
            quantity_text(request.quantity_milli),
        )));
    }
    StockMovement::leaving(
        MovementType::Adjust,
        request.item.subject_id,
        request.lot.as_ref().map(|lot| lot.subject_id),
        request.location.subject_id,
        request.quantity_milli,
        request.unit_cost,
        value_of(request.quantity_milli, request.unit_cost),
        &request.source,
        Some(request.reason),
        request.user,
    )
    .insert(&mut *tx)
    .await?;
    Ok(())
}

/// Reverses the `wastage` of a voided report's scrap (see the module).
///
/// # Errors
///
/// 422 for a quantity that is not above zero; a database error.
pub async fn void_waste(tx: &mut Transaction, request: WasteMaterial) -> Result {
    if request.quantity_milli <= 0 {
        return Err(refused("The quantity must be above zero."));
    }
    let unit_cost = i64::try_from(
        (i128::from(request.value) * 1_000 + i128::from(request.quantity_milli) / 2)
            / i128::from(request.quantity_milli),
    )
    .unwrap_or(0);
    let mut movement = StockMovement::moving(
        MovementType::Adjust,
        request.item.subject_id,
        request.lot.as_ref().map(|lot| lot.subject_id),
        None,
        None,
        request.quantity_milli,
        unit_cost,
        request.value,
        &request.source,
        request.user,
    );
    movement.reason = Some(request.reason);
    movement.insert(&mut *tx).await?;
    Ok(())
}

/// The module's [`ProductionStock`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryProduction;

impl ProductionStock for InventoryProduction {
    fn return_material<'a>(
        &'a self,
        tx: &'a mut Transaction,
        request: ReturnMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move { return_material(tx, request).await })
    }

    fn waste_material<'a>(
        &'a self,
        tx: &'a mut Transaction,
        request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move { waste_material(tx, request).await })
    }

    fn produce<'a>(
        &'a self,
        tx: &'a mut Transaction,
        request: ProduceOutput,
    ) -> ProductionFuture<'a, Option<Subject>> {
        Box::pin(async move { produce(tx, request).await })
    }

    fn output_available<'a>(
        &'a self,
        state: &'a AppState,
        item: &'a Subject,
        lot: Option<&'a Subject>,
        location: &'a Subject,
    ) -> ProductionFuture<'a, Option<i64>> {
        Box::pin(async move {
            output_available(&state.db, item, lot, location)
                .await
                .map(Some)
        })
    }

    fn void_output<'a>(
        &'a self,
        tx: &'a mut Transaction,
        request: VoidOutput,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move { void_output(tx, request).await })
    }

    fn void_waste<'a>(
        &'a self,
        tx: &'a mut Transaction,
        request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move { void_waste(tx, request).await })
    }
}
