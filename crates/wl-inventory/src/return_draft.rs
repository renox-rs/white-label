//! Drafting a Return to supplier for another module (wl-inventory §6,
//! `SupplierReturnDrafts`): 08 Auto asks for one when a supplier warranty
//! claim is raised for a failed part that was kept (08 §4.6 step 4). The return
//! is a `draft` with outcome *credit*, its goods kept for collection at the
//! location named; dispatching it (BL-STK-03) drafts the supplier debit note,
//! and the debit note's settlement credits it (BL-STK-05).

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{DraftReturn, LOCATION_SUBJECT_TYPE, StockFuture, SupplierReturnDrafts};

use crate::item::Item;
use crate::supplier_return::{NewReturn, NewReturnLine, create_from_stock};

/// Drafts the return of `request`: `None` when the catalogue has no item of that
/// code or the location is not a location.
///
/// # Errors
///
/// 422 for a quantity that is not above zero or a blank reason; the database's
/// errors.
pub async fn draft_credit(state: &AppState, request: DraftReturn) -> Result<Option<Subject>> {
    if request.location.subject_type != LOCATION_SUBJECT_TYPE {
        return Ok(None);
    }
    let Some(item) = Item::query()
        .where_(Item::CODE.eq(request.item_code.trim()))
        .first(&state.db)
        .await?
    else {
        return Ok(None);
    };
    let made = create_from_stock(
        &state.db,
        None,
        NewReturn {
            supplier: request.supplier,
            source: request.source,
            location_id: request.location.subject_id,
            lines: vec![
                NewReturnLine::of(item.id, request.quantity_milli, &request.reason).credit(),
            ],
        },
    )
    .await?;
    Ok(Some(made.subject()))
}

/// The module's [`SupplierReturnDrafts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryReturnDrafts;

impl SupplierReturnDrafts for InventoryReturnDrafts {
    fn draft_credit<'a>(
        &'a self,
        state: &'a AppState,
        request: DraftReturn,
    ) -> StockFuture<'a, Option<Subject>> {
        Box::pin(async move { draft_credit(state, request).await })
    }
}
