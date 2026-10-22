//! What the kitchen tells the shared modules about an order's lines
//! (06 F&B §7.1, `FNB.ACL.16`):
//!
//! - [`KitchenProgress`] is the kernel's `FulfilmentProgress`: which sale
//!   lines of an order the kitchen started. A cancellation of a started line
//!   needs an outcome (return to stock, scrap) and a manager (`SAL.ACL.05`).
//! - [`KitchenStock`] is the kernel's `SaleLineStockSource`: per sale line the
//!   location, the recipe per unit, the quantity, whether the kitchen's
//!   consumption was already posted, and the value. Without it a scrap of
//!   prepared food posts nothing and a discarded delivery records no wastage.
//!
//! A line has started when its ticket is in preparation or done. An order
//! with no ticket (one made by an import, or by a seed) counts every line as
//! started once the order is past `accepted`.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    LineComponent, SaleLineStock, SaleLineStockSource, StockFuture, ITEM_SUBJECT_TYPE,
    LOCATION_SUBJECT_TYPE,
};
use wl_kernel::sales::{
    FulfilmentProgress, SALES_ORDER_SUBJECT_TYPE, SalesFuture, SalesOrderId, order_desk,
};

use crate::catalogue::{MenuItem, Outlet};
use crate::order::{FnbOrder, FnbOrderStatus, SUBJECT_TYPE};
use crate::ticket::KitchenTicket;

/// The kitchen's [`FulfilmentProgress`].
pub struct KitchenProgress;

/// The sale lines of `order` the kitchen started, among `lines`.
pub(crate) async fn started(db: &Db, order: &FnbOrder, lines: &[i64]) -> Result<Vec<i64>> {
    let tickets = KitchenTicket::of_order(db, order.id).await?;
    if tickets.is_empty() {
        let past_accepted = !matches!(
            order.status,
            FnbOrderStatus::Placed | FnbOrderStatus::Accepted | FnbOrderStatus::Rejected
        );
        return Ok(if past_accepted { lines.to_vec() } else { Vec::new() });
    }
    Ok(lines
        .iter()
        .copied()
        .filter(|line| {
            tickets
                .iter()
                .any(|t| t.sale_line_id == *line && t.status.is_started())
        })
        .collect())
}

impl FulfilmentProgress for KitchenProgress {
    fn started_lines<'a>(
        &'a self,
        state: &'a AppState,
        order: SalesOrderId,
        lines: &'a [i64],
    ) -> SalesFuture<'a, Vec<i64>> {
        Box::pin(async move {
            match FnbOrder::of_sales_order(&state.db, order.0).await? {
                Some(found) => started(&state.db, &found, lines).await,
                None => Ok(Vec::new()),
            }
        })
    }
}

/// The kitchen's [`SaleLineStockSource`].
pub struct KitchenStock;

/// The F&B order an order subject names: the sales order or the F&B order
/// itself (a delivery's source).
async fn order_of(db: &Db, subject: &Subject) -> Result<Option<FnbOrder>> {
    match subject.subject_type.as_str() {
        SUBJECT_TYPE => FnbOrder::find(db, subject.subject_id).await,
        SALES_ORDER_SUBJECT_TYPE => FnbOrder::of_sales_order(db, subject.subject_id).await,
        _ => Ok(None),
    }
}

/// The stock side of the sale line `line` of `order`.
async fn stock_of(state: &AppState, order: &FnbOrder, line: i64) -> Result<Option<SaleLineStock>> {
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(None);
    };
    let Some(sold) = view.lines.iter().find(|l| l.id == line && l.is_item) else {
        return Ok(None);
    };
    let Some(item) = MenuItem::by_code(&state.db, &sold.item_code).await? else {
        return Ok(None);
    };
    let outlet = match order.outlet_id {
        Some(id) => Outlet::find(&state.db, id).await?,
        None => None,
    };
    let Some(location) = outlet.and_then(|outlet| outlet.location_id) else {
        return Ok(None);
    };
    let components = item
        .recipe(&state.db)
        .await?
        .into_iter()
        .map(|recipe| LineComponent {
            item: Subject::new(ITEM_SUBJECT_TYPE, recipe.item_id),
            per_unit_milli: recipe.per_unit_milli,
        })
        .collect();
    let quantity = sold.quantity_milli;
    let value = i64::try_from(i128::from(item.unit_cost) * i128::from(quantity) / 1_000)
        .unwrap_or(item.unit_cost);
    Ok(Some(SaleLineStock {
        location: Subject::new(LOCATION_SUBJECT_TYPE, location),
        components,
        quantity_milli: quantity,
        // Posted by the kitchen, or already posted as wastage by the
        // delivery's discard: a scrap then posts no second movement.
        consumed: order.consumption_posted || order.wastage_posted,
        value,
    }))
}

impl SaleLineStockSource for KitchenStock {
    fn of_line<'a>(
        &'a self,
        state: &'a AppState,
        order: &'a Subject,
        line: i64,
    ) -> StockFuture<'a, Option<SaleLineStock>> {
        Box::pin(async move {
            match order_of(&state.db, order).await? {
                Some(found) => stock_of(state, &found, line).await,
                None => Ok(None),
            }
        })
    }
}
