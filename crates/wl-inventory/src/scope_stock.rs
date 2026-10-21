//! What wl-inventory does for a vertical when a scope stops (02
//! Construction §7.1 *Project* `terminate`): the module's
//! [`ScopeStock`](wl_kernel::inventory::ScopeStock). The scope is the project
//! (or building, branch) a location is in ([`Location::scope_subject`]).
//!
//! - `open_orders`: the purchase orders delivering to a location of the scope
//!   that are `issued` or `partially_received`. Nothing cancels them here: the
//!   supplier must be told, so the vertical gives procurement a task to
//!   `cancel` or `short_close` each;
//! - `cancel_open_requests`: the system cancels the purchase requests of those
//!   locations that are `draft`, `submitted` or `approved` (no order issued),
//!   with the reason;
//! - `open_stock_counts`: one stock count for each location of the scope that
//!   has none open (`STK.CNT.01`).

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::inventory::{OpenOrder, ScopeFuture, ScopeStock};
use wl_kernel::{PartyId, Subject};

use crate::goods_receipt::GoodsReceipt;
use crate::location::Location;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderStatus, lines_of};
use crate::purchase_request::{self, PurchaseRequest, PurchaseRequestStatus};
use crate::stock_count::{self, StockCount, StockCountStatus};

/// The ids of the locations in `scope`.
async fn location_ids(db: &Db, scope: &Subject) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Location::query()
        .get(db)
        .await?
        .into_iter()
        .filter(|location| location.scope_subject().as_ref() == Some(scope))
        .map(|location| location.id)
        .collect();
    ids.sort_unstable();
    Ok(ids)
}

/// The module's [`ScopeStock`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryScopeStock;

impl ScopeStock for InventoryScopeStock {
    fn open_orders<'a>(
        &'a self,
        state: &'a AppState,
        scope: &'a Subject,
    ) -> ScopeFuture<'a, Vec<OpenOrder>> {
        Box::pin(async move {
            let ids = location_ids(&state.db, scope).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let mut orders = PurchaseOrder::query()
                .where_(PurchaseOrder::LOCATION_ID.is_in(ids))
                .where_(PurchaseOrder::STATUS.is_in([
                    PurchaseOrderStatus::Issued,
                    PurchaseOrderStatus::PartiallyReceived,
                ]))
                .get(&state.db)
                .await?;
            orders.sort_by_key(|order| order.id);
            let mut open = Vec::new();
            for order in orders {
                let received = lines_of(&state.db, order.id)
                    .await?
                    .iter()
                    .any(|line| line.accepted_qty_milli > 0);
                open.push(OpenOrder {
                    order: order.subject(),
                    number: order.number.clone(),
                    supplier: PartyId(order.supplier_party_id),
                    received,
                });
            }
            Ok(open)
        })
    }

    fn cancel_open_requests<'a>(
        &'a self,
        state: &'a AppState,
        scope: &'a Subject,
        reason: &'a str,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let ids = location_ids(&state.db, scope).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let mut requests = PurchaseRequest::query()
                .where_(PurchaseRequest::LOCATION_ID.is_in(ids))
                .where_(PurchaseRequest::STATUS.is_in(PurchaseRequestStatus::OPEN))
                .get(&state.db)
                .await?;
            requests.sort_by_key(|request| request.id);
            let mut cancelled = Vec::new();
            for request in requests {
                let _outcome = request
                    .transition(purchase_request::AUTO_CANCEL)
                    .by_system()
                    .reason(reason)
                    .run(state)
                    .await
                    .map_err(renox::state_machine::TransitionError::into_error)?;
                cancelled.push(request.subject());
            }
            Ok(cancelled)
        })
    }

    fn open_stock_counts<'a>(
        &'a self,
        state: &'a AppState,
        scope: &'a Subject,
        reason: &'a str,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let mut opened = Vec::new();
            for id in location_ids(&state.db, scope).await? {
                let (count, new) = stock_count::open_for(&state.db, id, reason, scope).await?;
                if new {
                    opened.push(count.subject());
                }
            }
            Ok(opened)
        })
    }

    fn has_goods_receipts<'a>(
        &'a self,
        state: &'a AppState,
        scope: &'a Subject,
    ) -> ScopeFuture<'a, bool> {
        Box::pin(async move {
            let ids = location_ids(&state.db, scope).await?;
            if ids.is_empty() {
                return Ok(false);
            }
            Ok(GoodsReceipt::query()
                .where_(GoodsReceipt::LOCATION_ID.is_in(ids))
                .first(&state.db)
                .await?
                .is_some())
        })
    }

    fn open_counts<'a>(
        &'a self,
        state: &'a AppState,
        scope: &'a Subject,
    ) -> ScopeFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let ids = location_ids(&state.db, scope).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let mut counts = StockCount::query()
                .where_(StockCount::LOCATION_ID.is_in(ids))
                .where_(StockCount::STATUS.is_in(StockCountStatus::UNFINISHED))
                .get(&state.db)
                .await?;
            counts.sort_by_key(|count| count.id);
            Ok(counts.iter().map(StockCount::subject).collect())
        })
    }
}
