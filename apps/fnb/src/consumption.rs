//! The kitchen's consumption of a recipe (06 F&B §4.4 recall row, §7.1
//! *Order* `complete`; the base for BL-FNB-13).
//!
//! A completed order consumes what its menu items are made of from the stock
//! location of its outlet, as **theoretical** consumption: the recipe says how
//! much a unit uses, nobody scans the lot. The stock module posts it against
//! the lots on hand (first expiry first) and records a pending issue for
//! what is short; the movement is marked theoretical, so a recall trace lists
//! the order as *possible* for a recalled lot, never *confirmed*
//! (`wl-inventory` §4 *Recall*).
//!
//! The movement is dated at the order's time (when it was placed), the time a
//! trace compares with the window a lot was on hand. It is posted once per
//! order (`FnbOrder.consumption_posted`) and only when the setting
//! [`recipe_consumption`](crate::settings::FnbSettings::recipe_consumption) is
//! on: without the recipe lines of the menu there is nothing to consume.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{
    ConsumeRequest, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE, stock_service,
};
use wl_kernel::sales::order_desk;

use crate::catalogue::{MenuItem, Outlet};
use crate::order::FnbOrder;
use crate::settings::settings;

/// Consumes the recipe of every item line of order `id`, once. Returns how many
/// ingredient movements or pending issues it asked the stock module for.
///
/// # Errors
///
/// Database errors and the stock and sales modules'.
pub async fn post(state: &AppState, id: i64) -> Result<usize> {
    if !settings(state).recipe_consumption {
        return Ok(0);
    }
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(0);
    };
    if order.consumption_posted {
        return Ok(0);
    }
    let outlet = match order.outlet_id {
        Some(outlet) => Outlet::find(&state.db, outlet).await?,
        None => None,
    };
    let Some(location) = outlet.and_then(|outlet| outlet.location_id) else {
        return Ok(0);
    };
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(0);
    };
    let mut asked = 0;
    for line in view.lines.iter().filter(|line| line.is_item) {
        // What was cooked: the quantity ordered less what was cancelled, handed
        // over or not (an order that is already completed consumed it too).
        let quantity = line.quantity_milli - line.cancelled_milli;
        if quantity <= 0 {
            continue;
        }
        let Some(item) = MenuItem::by_code(&state.db, &line.item_code).await? else {
            continue;
        };
        for component in item.recipe(&state.db).await? {
            let used =
                i64::try_from(i128::from(component.per_unit_milli) * i128::from(quantity) / 1_000)
                    .unwrap_or(0);
            if used <= 0 {
                continue;
            }
            let mut request = ConsumeRequest::new(
                Subject::new(ITEM_SUBJECT_TYPE, component.item_id),
                Subject::new(LOCATION_SUBJECT_TYPE, location),
                used,
                order.subject(),
            )
            .theoretical();
            if let Some(at) = order.created_at {
                request = request.at(at);
            }
            stock_service(state).consume(state, request).await?;
            asked += 1;
        }
    }
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(&state.db, &[("consumption_posted", &true)])
        .await?;
    Ok(asked)
}
