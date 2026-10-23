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
//! Decisions where the spec is silent: the output lot is numbered by the
//! caller (`F-WO-2026-0001`); reservations of output lots for order lines
//! are not kept by this module yet, so `release_reservation` answers `false`
//! (the dispatch pick reserves, not the work order).

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::LOT_SUBJECT_TYPE;
use wl_kernel::production::{
    ProduceOutput, ProductionFuture, ProductionStock, ReturnMaterial, WasteMaterial,
};

use crate::item::{Item, value_of};
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
}
