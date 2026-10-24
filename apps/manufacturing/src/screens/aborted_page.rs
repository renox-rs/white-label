//! The aborted work orders report (`GET /manufacturing/aborted`, BL-MFG-01
//! *Surfaces*): per aborted work order the reason, who asked and who
//! approved, when it stopped and, to the people who may see costs
//! (`work_orders.cost_view`, `FIN.ACL.08`), the scrap value and the loss the
//! stop cost. A work order is listed to the people who may see it; the
//! figures are left out of the JSON too for the others.
//!
//! The list is the standard grid over the aborted work orders the person may
//! see (the scope is in the query; search on the number, the item and the
//! reason; filters by date and requester). The scrap value and the loss are
//! columns only when the person may see the costs of every order listed.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;

use super::{Texts, aed, exported, ids_or_none, time_text, user_text, with_item_search};
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
    rows: GridPage<WorkOrder>,
}

/// The aborted work orders grid. `costs` adds the scrap value and the loss.
pub(crate) fn grid(lang: &Lang, costs: bool, can_export: bool) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.aborted.{key}"));
    let mut grid = Grid::new("manufacturing-aborted")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &tx("order"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/manufacturing/work-orders/{id}"),
        )
        .column(Column::custom("item", &t(lang, "manufacturing.screen.fact.item")).mobile())
        .column(
            Column::text("abort_reason", &tx("reason"))
                .mobile()
                .searchable()
                .wrap(),
        )
        .column(Column::datetime("aborted_at", &tx("stopped")).mobile())
        .column(Column::related(
            "requested_by",
            &tx("requested_by"),
            "users",
            "abort_requested_by",
            "name",
        ))
        .column(Column::related(
            "approved_by",
            &tx("approved_by"),
            "users",
            "abort_approved_by",
            "name",
        ));
    if costs {
        grid = grid
            .column(Column::money(
                "abort_scrap_value",
                &wl_kernel::money_label(&tx("scrap")),
            ))
            .column(
                Column::money("abort_loss", &wl_kernel::money_label(&tx("loss")))
                    .summary(renox::grid::Summary::Sum),
            );
    }
    let grid = grid
        .sort_by("-aborted_at")
        .row_url("/manufacturing/work-orders/{id}")
        .cards_on_mobile()
        .empty_state(&tx("empty"), Some(&tx("empty_text")));
    let grid = with_item_search(grid, lang, "item_id", false);
    if can_export { grid.exports() } else { grid }
}

/// `GET /manufacturing/aborted`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let visible: Vec<WorkOrder> = WorkOrder::query()
        .where_(WorkOrder::STATUS.eq(WorkOrderStatus::Aborted))
        .order_by_desc(WorkOrder::ABORTED_AT)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|order| order.may_see(&user))
        .collect();
    if !wl_kernel::negotiate::wants_html(&headers) {
        let unknown = t(&lang, "manufacturing.screen.not_set");
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
    // The money columns only for who may see the cost of every order listed.
    let costs = !visible.is_empty()
        && visible
            .iter()
            .all(|order| order.may(&user, permissions::WORK_ORDERS_COST_VIEW));
    let can_export = user.has_permission_anywhere(permissions::WORK_ORDERS_CLOSE)
        || user.has_permission_anywhere(permissions::WORK_ORDERS_ABORT_APPROVE);
    let grid = grid(&lang, costs, can_export);
    let query = WorkOrder::query()
        .where_(WorkOrder::ID.is_in(ids_or_none(visible.iter().map(|order| order.id).collect())));
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    for order in page.items() {
        extra.insert(
            order.id,
            json!({ "item": super::work_order_page::item_text(&state, &lang, order.item_id).await }),
        );
    }
    let page = page.extend(|order| extra.get(&order.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(AbortedPage {
        tx: screen_texts(&lang),
        rows: page,
    })
    .into_response())
}
