//! The aborted work orders report (`GET /manufacturing/aborted`, BL-MFG-01
//! *Surfaces*): per aborted work order the reason, who asked and who
//! approved, when it stopped and, to the people who may see costs
//! (`work_orders.cost_view`, `FIN.ACL.08`), the scrap value and the loss the
//! stop cost. A work order is listed to the people who may see it; the
//! figures are left out of the JSON too for the others. The page is a
//! `renox::grid` searched by number and reason; the figures are custom
//! columns, filled per row, so no export carries them.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::Serialize;

use super::{Texts, aed, grids, time_text, user_text};
use crate::permissions;
use crate::texts::{screen_texts, t};
use crate::work_order::{WorkOrder, WorkOrderStatus};

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
    stopped: GridPage<WorkOrder>,
}

/// The name of the user in the work order's column `fk`, as SQL.
fn user_sql(fk: &str) -> String {
    format!("(SELECT \"users\".\"name\" FROM \"users\" WHERE \"users\".\"id\" = {{T}}.\"{fk}\")")
}

/// The aborted work orders grid: the number first and frozen, the item, the
/// reason, when it stopped, who asked and who approved, and the scrap value
/// and loss (custom: empty for who may not see costs); last stopped first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    grids::baseline(Grid::new("manufacturing-aborted"))
        .title(&text("aborted.title"))
        .column(
            Column::text("number", &text("aborted.order"))
                .frozen()
                .mobile()
                .searchable()
                .link("/manufacturing/work-orders/{id}"),
        )
        .column(Column::custom("item", &text("grid.item")).mobile())
        .column(
            Column::text("abort_reason", &text("grid.reason"))
                .searchable()
                .mobile(),
        )
        .column(Column::datetime("aborted_at", &text("aborted.stopped")))
        .column(
            Column::text("requested_by", &text("aborted.requested_by"))
                .computed(&user_sql("abort_requested_by"))
                .searchable(),
        )
        .column(
            Column::text("approved_by", &text("aborted.approved_by"))
                .computed(&user_sql("abort_approved_by"))
                .searchable(),
        )
        .column(Column::custom("scrap", &text("grid.scrap")).hidden())
        .column(Column::custom("loss", &text("aborted.loss")))
        .sort_by("-aborted_at")
        .row_url("/manufacturing/work-orders/{id}")
        .empty_state(&text("aborted.empty"), Some(&text("aborted.empty_text")))
}

/// `GET /manufacturing/aborted`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let unknown = t(&lang, "manufacturing.screen.not_set");
    let mut visible = Vec::new();
    for order in WorkOrder::query()
        .where_(WorkOrder::STATUS.eq(WorkOrderStatus::Aborted))
        .order_by_desc(WorkOrder::ABORTED_AT)
        .get(&state.db)
        .await?
    {
        if order.may_see(&user) {
            visible.push(order);
        }
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for order in &visible {
            let costs = order.may(&user, permissions::WORK_ORDERS_COST_VIEW);
            let figure = |fils: Option<i64>| match fils {
                Some(fils) if costs => aed(fils),
                _ => String::new(),
            };
            rows.push(Row {
                url: super::work_order_page::url(order.id),
                number: order.number.clone(),
                item: super::work_order_page::item_text(&state, &lang, order.item_id).await,
                reason: order.abort_reason.clone().unwrap_or_default(),
                stopped: order.aborted_at.map_or_else(String::new, time_text),
                requested_by: user_text(&state, order.abort_requested_by, &unknown).await,
                approved_by: user_text(&state, order.abort_approved_by, &unknown).await,
                scrap: figure(order.abort_scrap_value),
                loss: figure(order.abort_loss),
            });
        }
        return Ok(Json(&rows).into_response());
    }
    // The figures are custom columns: an export never carries them.
    let grid = grids::exporting(grid(&lang), true);
    let query = WorkOrder::query().where_in("id", visible.iter().map(|order| order.id));
    if let Some(file) = grids::export_file(&grid, true, query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    for order in page.items() {
        let costs = order.may(&user, permissions::WORK_ORDERS_COST_VIEW);
        let figure = |fils: Option<i64>| match fils {
            Some(fils) if costs => aed(fils),
            _ => String::new(),
        };
        extra.insert(
            order.id,
            json!({
                "item": super::work_order_page::item_text(&state, &lang, order.item_id).await,
                "scrap": figure(order.abort_scrap_value),
                "loss": figure(order.abort_loss),
            }),
        );
    }
    let stopped = page.extend(|order| extra.get(&order.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(AbortedPage {
        tx: screen_texts(&lang),
        stopped,
    })
    .into_response())
}
