//! What a vertical does with a failed lot, through the kernel's
//! [`LotDisposal`] (07 Manufacturing §7.1 *NCR*, BL-MFG-08) and
//! [`DispatchLots`] (BL-MFG-13).
//!
//! - **Facts** of a lot: its item, status, what is left and where, the
//!   supplier of the goods receipt it came from and the receipt line's cost
//!   and order line, else the item's average cost. A lot from a goods receipt
//!   has a *supplier lot*: the number printed on the pack; a lot made by a
//!   work order or opened by hand has none.
//! - **`create_return`**: a draft Return to supplier kept for collection,
//!   from the location the lot is at ([`crate::supplier_return::create_from_stock`]),
//!   one line of the NCR's quantity at the receipt's cost with the NCR as
//!   its source. Refused (422) for a lot with no supplier lot, a quantity
//!   above zero that is not at the location (less what other draft returns
//!   hold aside).
//! - **`scrap`**: the whole remaining quantity of a held or expired lot is
//!   the lot's `discard`; any other quantity is a `wastage` movement out of
//!   the lot's locations (the NCR's `STK.ACL.02` approval was given before).
//! - **`release`** and **`close_out`**: the system's `release` of a lot
//!   (*use as is*), and the `reject` of a lot a return emptied.
//! - **`SupplierReturnCancelled`**: emitted after a draft return is cancelled.
//!
//! Decision: the quarantine location of the spec is the location the lot's
//! stock is at: a lot is moved to quarantine by its status, not by a place.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::inventory::{
    DISPATCH_LINE_SUBJECT_TYPE, DISPATCH_SUBJECT_TYPE, GOODS_RECEIPT_SUBJECT_TYPE,
    ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE, ReturnOutcome,
};
use wl_kernel::lot_disposal::{
    DispatchLots, DispatchedLot, LotDisposal, LotFacts, LotFuture, ReturnLot, ScrapLot, Scrapped,
    SupplierReturnCancelled,
};
use wl_kernel::{PartyId, Subject};

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine};
use crate::goods_receipt::{GoodsReceipt, GoodsReceiptLine};
use crate::item::{Item, quantity_text, value_of};
use crate::location::quarantine_location;
use crate::lot::{self, Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::supplier_return::{self, NewReturn, NewReturnLine, SupplierReturn, WantedOutcome};
use crate::texts::said;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The facts of lot `lot_id`, `None` when there is no such lot.
///
/// # Errors
///
/// A database error.
pub async fn facts_of(db: &Db, lot_id: i64) -> Result<Option<LotFacts>> {
    let Some(lot) = Lot::find(db, lot_id).await? else {
        return Ok(None);
    };
    let item = Item::find_or_404(db, lot.item_id).await?;
    let mut supplier = None;
    let mut receipt = None;
    let mut unit_cost = item.average_cost;
    let mut po_line = None;
    if let Some(source) = lot.source()
        && source.subject_type == GOODS_RECEIPT_SUBJECT_TYPE
        && let Some(found) = GoodsReceipt::find(db, source.subject_id).await?
    {
        supplier = Some(PartyId(found.supplier_party_id));
        receipt = Some(source);
        if let Some(line) = GoodsReceiptLine::query()
            .where_(GoodsReceiptLine::GOODS_RECEIPT_ID.eq(found.id))
            .where_(GoodsReceiptLine::LOT_ID.eq(Some(lot.id)))
            .first(db)
            .await?
        {
            unit_cost = line.unit_cost;
            po_line = line.po_line_id;
        }
    }
    let produced_by = lot
        .source()
        .filter(|source| source.subject_type != GOODS_RECEIPT_SUBJECT_TYPE);
    let places = balance::lot_locations(db, lot.id).await?;
    let quantity_milli = places.iter().map(|(_, quantity)| quantity).sum();
    let biggest = places.iter().max_by_key(|(_, quantity)| *quantity).copied();
    let supplier_lot = if receipt.is_some() {
        Some(
            lot.supplier_lot
                .clone()
                .unwrap_or_else(|| lot.number.clone()),
        )
    } else {
        lot.supplier_lot.clone()
    };
    Ok(Some(LotFacts {
        lot: Subject::new(LOT_SUBJECT_TYPE, lot.id),
        item: Subject::new(ITEM_SUBJECT_TYPE, lot.item_id),
        number: lot.number.clone(),
        supplier_lot,
        status: lot.status.as_str().to_owned(),
        supplier,
        receipt,
        produced_by,
        quantity_milli,
        location: biggest.map(|(id, _)| Subject::new(LOCATION_SUBJECT_TYPE, id)),
        location_milli: biggest.map_or(0, |(_, quantity)| quantity),
        unit_cost,
        average_cost: item.average_cost,
        po_line,
    }))
}

/// Creates the draft return of `request` (see the module).
///
/// # Errors
///
/// 422 as the module says; a database error.
pub async fn create_return(state: &AppState, request: ReturnLot) -> Result<Subject> {
    let facts = facts_of(&state.db, request.lot.subject_id)
        .await?
        .ok_or(Error::NotFound)?;
    let (Some(supplier), Some(_)) = (facts.supplier, facts.supplier_lot.as_ref()) else {
        return Err(refused(said("wl_inventory.lot.no_supplier_lot", &[])));
    };
    let Some(location) = facts.location.as_ref() else {
        return Err(refused(said("wl_inventory.lot.nothing_left", &[])));
    };
    let reserved = supplier_return::reserved_milli(
        &state.db,
        facts.item.subject_id,
        location.subject_id,
        Some(request.lot.subject_id),
    )
    .await?;
    let free = facts.location_milli - reserved;
    if request.quantity_milli <= 0 || request.quantity_milli > free {
        return Err(refused(said(
            "wl_inventory.lot.not_at_location",
            &[
                ("available", &quantity_text(free.max(0))),
                ("quantity", &quantity_text(request.quantity_milli)),
            ],
        )));
    }
    let wanted = match request.outcome {
        ReturnOutcome::Credit => WantedOutcome::Credit,
        ReturnOutcome::Replace => WantedOutcome::Replace,
    };
    let line = NewReturnLine {
        item_id: facts.item.subject_id,
        lot_id: Some(request.lot.subject_id),
        quantity_milli: request.quantity_milli,
        reason: request.reason.clone(),
        wanted_outcome: wanted,
        po_line_id: facts.po_line,
        unit_cost: Some(facts.unit_cost),
    };
    // The goods go to the quarantine location (`STK.ACL.06` applies at the
    // dispatch), as the stock hold's `return_to_supplier` does.
    let mut tx = state.db.begin().await?;
    let quarantine = quarantine_location(&mut tx).await?;
    if quarantine != location.subject_id {
        let item = Item::find_or_404(&mut tx, facts.item.subject_id).await?;
        let mut moved = StockMovement::moving(
            MovementType::TransferOut,
            item.id,
            Some(request.lot.subject_id),
            Some(location.subject_id),
            Some(quarantine),
            request.quantity_milli,
            item.average_cost,
            value_of(request.quantity_milli, item.average_cost),
            &request.ncr,
            request.user,
        );
        moved.reason = Some(request.reason.clone());
        moved.insert(&mut tx).await?;
    }
    tx.commit().await?;
    let made = supplier_return::create_from_stock(
        &state.db,
        request.user,
        NewReturn {
            supplier,
            source: request.ncr,
            location_id: quarantine,
            lines: vec![line],
        },
    )
    .await?;
    Ok(made.subject())
}

/// Writes the quantity of `request` off (see the module).
///
/// # Errors
///
/// 422 when the lot does not hold the quantity; a database error.
pub async fn scrap(state: &AppState, request: ScrapLot) -> Result<Scrapped> {
    let lot = Lot::find_or_404(&state.db, request.lot.subject_id).await?;
    let item = Item::find_or_404(&state.db, lot.item_id).await?;
    let places = balance::lot_locations(&state.db, lot.id).await?;
    let left: i64 = places.iter().map(|(_, quantity)| quantity).sum();
    if request.quantity_milli <= 0 || request.quantity_milli > left {
        return Err(refused(said(
            "wl_inventory.lot.not_at_location",
            &[
                ("available", &quantity_text(left.max(0))),
                ("quantity", &quantity_text(request.quantity_milli)),
            ],
        )));
    }
    let whole = request.quantity_milli == left;
    if whole && matches!(lot.status, LotStatus::OnHold | LotStatus::Expired) {
        let outcome = lot
            .transition(lot::DISCARD)
            .by_system()
            .reason(request.reason.clone())
            .run(state)
            .await
            .map_err(TransitionError::into_error)?;
        outcome.emit(state).await?;
        return Ok(Scrapped {
            value: value_of(left, item.average_cost),
            discarded: true,
        });
    }
    let mut tx = state.db.begin().await?;
    let mut to_go = request.quantity_milli;
    let mut value = 0;
    for (place, quantity) in places {
        if to_go == 0 {
            break;
        }
        let part = to_go.min(quantity);
        let worth = value_of(part, item.average_cost);
        let mut moved = StockMovement::leaving(
            MovementType::Wastage,
            item.id,
            Some(lot.id),
            place,
            part,
            item.average_cost,
            worth,
            &request.ncr,
            Some(request.reason.clone()),
            request.user,
        );
        moved.insert(&mut tx).await?;
        to_go -= part;
        value += worth;
    }
    tx.commit().await?;
    Ok(Scrapped {
        value,
        discarded: false,
    })
}

/// Releases `lot_id` as the system; whether it is released afterwards.
///
/// # Errors
///
/// A database error.
pub async fn release(state: &AppState, lot_id: i64) -> Result<bool> {
    let lot = Lot::find_or_404(&state.db, lot_id).await?;
    match lot.status {
        LotStatus::Released => return Ok(true),
        LotStatus::Quarantine | LotStatus::OnHold => {}
        _ => return Ok(false),
    }
    let outcome = lot
        .transition(lot::RELEASE)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    outcome.emit(state).await?;
    Ok(true)
}

/// Rejects a lot in quarantine or on hold that a return emptied.
///
/// # Errors
///
/// A database error.
pub async fn close_out(state: &AppState, lot_id: i64) -> Result<bool> {
    let lot = Lot::find_or_404(&state.db, lot_id).await?;
    if !matches!(lot.status, LotStatus::Quarantine | LotStatus::OnHold)
        || balance::lot_left(&state.db, lot.id).await? > 0
    {
        return Ok(false);
    }
    let outcome = lot
        .transition(lot::REJECT)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    outcome.emit(state).await?;
    Ok(true)
}

/// The service the kernel hands to the verticals.
pub struct InventoryLots;

impl LotDisposal for InventoryLots {
    fn facts<'a>(
        &'a self,
        state: &'a AppState,
        lot: &'a Subject,
    ) -> LotFuture<'a, Option<LotFacts>> {
        Box::pin(async move {
            if lot.subject_type != LOT_SUBJECT_TYPE {
                return Ok(None);
            }
            facts_of(&state.db, lot.subject_id).await
        })
    }

    fn create_return<'a>(
        &'a self,
        state: &'a AppState,
        request: ReturnLot,
    ) -> LotFuture<'a, Option<Subject>> {
        Box::pin(async move { create_return(state, request).await.map(Some) })
    }

    fn return_status<'a>(
        &'a self,
        state: &'a AppState,
        supplier_return: &'a Subject,
    ) -> LotFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok(SupplierReturn::find(&state.db, supplier_return.subject_id)
                .await?
                .map(|found| found.status.as_str().to_owned()))
        })
    }

    fn scrap<'a>(
        &'a self,
        state: &'a AppState,
        request: ScrapLot,
    ) -> LotFuture<'a, Option<Scrapped>> {
        Box::pin(async move { scrap(state, request).await.map(Some) })
    }

    fn release<'a>(&'a self, state: &'a AppState, lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async move { release(state, lot.subject_id).await })
    }

    fn close_out<'a>(&'a self, state: &'a AppState, lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async move { close_out(state, lot.subject_id).await })
    }
}

/// The lots behind a dispatch line or a dispatch.
pub struct InventoryDispatchLots;

impl InventoryDispatchLots {
    fn of_line(line: &DispatchLine) -> DispatchedLot {
        DispatchedLot {
            line: line.subject(),
            item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
            lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
            quantity_milli: line.quantity_milli,
        }
    }
}

impl DispatchLots for InventoryDispatchLots {
    fn lots_of<'a>(
        &'a self,
        state: &'a AppState,
        origin: &'a Subject,
    ) -> LotFuture<'a, Vec<DispatchedLot>> {
        Box::pin(async move {
            if origin.subject_type == DISPATCH_LINE_SUBJECT_TYPE {
                return Ok(DispatchLine::find(&state.db, origin.subject_id)
                    .await?
                    .iter()
                    .map(Self::of_line)
                    .collect());
            }
            if origin.subject_type == DISPATCH_SUBJECT_TYPE
                && let Some(dispatch) = Dispatch::find(&state.db, origin.subject_id).await?
            {
                return Ok(DispatchLine::query()
                    .where_(DispatchLine::DISPATCH_ID.eq(dispatch.id))
                    .order_by(DispatchLine::POSITION)
                    .get(&state.db)
                    .await?
                    .iter()
                    .map(Self::of_line)
                    .collect());
            }
            Ok(Vec::new())
        })
    }
}

/// Provides the services to the kernel.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::lot_disposal::provide_lot_disposal(app, InventoryLots);
    wl_kernel::lot_disposal::provide_dispatch_lots(app, InventoryDispatchLots);
}

/// Tells the verticals that draft return `ret` was cancelled by a user
/// ([`SupplierReturnCancelled`]). Emitted by the cancel service, not by a
/// listener of the transition: a `run` followed by `emit` announces the
/// transition twice.
///
/// # Errors
///
/// The listeners' errors.
pub(crate) async fn announce_cancelled(state: &AppState, ret: &SupplierReturn) -> Result {
    state
        .emit(SupplierReturnCancelled {
            supplier_return: ret.subject(),
            supplier: PartyId(ret.supplier_party_id),
            source: ret.source(),
            reason: ret.cancel_reason.clone(),
        })
        .await
}
