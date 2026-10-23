//! The aborted work orders report (`GET /manufacturing/aborted`, BL-MFG-01
//! *Surfaces*): per aborted work order the reason, who asked and who
//! approved, when it stopped and, to the people who may see costs
//! (`work_orders.cost_view`, `FIN.ACL.08`), the scrap value and the loss the
//! stop cost. A work order is listed to the people who may see it; the
//! figures are left out of the JSON too for the others.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;

use super::{Texts, aed, time_text, user_text};
use crate::permissions;
use crate::texts::{screen_texts, t};
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/aborted";

/// One aborted work order.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    url: String,
    number: String,
    item: String,
    reason: String,
    stopped: String,
    requested_by: String,
    approved_by: String,
    /// The scrap value; empty for who may not see costs.
    scrap: String,
    /// The loss; empty for who may not see costs.
    loss: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/aborted.html")]
pub(crate) struct AbortedPage {
    tx: Texts,
    rows: Vec<Row>,
}

/// `GET /manufacturing/aborted`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let unknown = t(&lang, "manufacturing.screen.not_set");
    let mut rows = Vec::new();
    for order in WorkOrder::query()
        .where_(WorkOrder::STATUS.eq(WorkOrderStatus::Aborted))
        .order_by_desc(WorkOrder::ABORTED_AT)
        .get(&state.db)
        .await?
    {
        if !order.may_see(&user) {
            continue;
        }
        let costs = order.may(&user, permissions::WORK_ORDERS_COST_VIEW);
        let figure = |fils: Option<i64>| match fils {
            Some(fils) if costs => aed(fils),
            _ => String::new(),
        };
        rows.push(Row {
            url: super::work_order_page::url(order.id),
            number: order.number.clone(),
            item: super::work_order_page::item_text(&lang, order.item_id),
            reason: order.abort_reason.clone().unwrap_or_default(),
            stopped: order.aborted_at.map_or_else(String::new, time_text),
            requested_by: user_text(&state, order.abort_requested_by, &unknown).await,
            approved_by: user_text(&state, order.abort_approved_by, &unknown).await,
            scrap: figure(order.abort_scrap_value),
            loss: figure(order.abort_loss),
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    Ok(View::typed(AbortedPage {
        tx: screen_texts(&lang),
        rows,
    })
    .into_response())
}
