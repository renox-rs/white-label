//! The work order list (`GET /manufacturing/work-orders`, `docs/UX.md` §2.1)
//! and its *New work order* sheet.
//!
//! The list is the standard grid: the work orders of the plants and lines the
//! person works for (the scope is in the query, not in the page), searched on
//! the number and the line, sorted, filtered by status, period and quantity
//! and paged. `?status=<key>` (`open` is every state before `completed`) still
//! narrows the query, so the links of the dashboard tiles keep working. The
//! item and the source come from the stock and sales contracts and are drawn
//! as custom cells.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;

use super::{Texts, exported, ids_or_none, period_text, quantity_text};
use crate::permissions;
use crate::plant::{Line, Plant};
use crate::texts::{screen_texts, t, word};
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// The address of the list.
pub(crate) const URL: &str = "/manufacturing/work-orders";

/// The statuses in the order of the lifecycle.
const STATUSES: [WorkOrderStatus; 8] = [
    WorkOrderStatus::Planned,
    WorkOrderStatus::Released,
    WorkOrderStatus::InProgress,
    WorkOrderStatus::OnHold,
    WorkOrderStatus::Completed,
    WorkOrderStatus::Closed,
    WorkOrderStatus::Cancelled,
    WorkOrderStatus::Aborted,
];

/// The badge tone of a work order status.
pub(crate) fn status_kind(status: WorkOrderStatus) -> &'static str {
    match status {
        WorkOrderStatus::Completed | WorkOrderStatus::Closed => "success",
        WorkOrderStatus::OnHold => "warning",
        WorkOrderStatus::Aborted => "error",
        WorkOrderStatus::Planned | WorkOrderStatus::Cancelled => "info",
        WorkOrderStatus::Released | WorkOrderStatus::InProgress => "info",
    }
}

/// A work order of the list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    id: i64,
    url: String,
    number: String,
    item: String,
    line: String,
    quantity: String,
    produced: String,
    period: String,
    source: String,
    status: String,
    status_kind: &'static str,
}

/// What the *New work order* sheet needs.
#[derive(Debug, Serialize, Clone, Default)]
pub(crate) struct NewForm {
    plants: Vec<(String, String)>,
    plant_selected: String,
    lines: Vec<(String, String)>,
    line_selected: String,
    today: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/work_orders.html")]
pub(crate) struct WorkOrdersPage {
    tx: Texts,
    rows: GridPage<WorkOrder>,
    all_url: String,
    open_url: String,
    open_now: bool,
    can_create: bool,
    form: NewForm,
}

/// The work orders grid: search on the number and the line, filters by status,
/// line, period and quantity, the quantities in pieces of the item (three
/// decimals). The item and the source are drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let statuses: Vec<(String, String)> = STATUSES
        .iter()
        .map(|status| {
            (
                status.as_str().to_owned(),
                word(lang, "status", status.as_str()),
            )
        })
        .collect();
    let grid = Grid::new("manufacturing-work-orders")
        .title(&t(lang, "manufacturing.screen.orders.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &t(lang, "manufacturing.screen.orders.order"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/manufacturing/work-orders/{id}"),
        )
        .column(Column::custom("item", &t(lang, "manufacturing.screen.fact.item")).mobile())
        .column(
            Column::related(
                "line",
                &t(lang, "manufacturing.screen.orders.line"),
                "manufacturing_lines",
                "line_id",
                "code",
            )
            .searchable(),
        )
        .column(
            Column::quantity(
                "quantity_milli",
                &t(lang, "manufacturing.screen.orders.planned_qty"),
            )
            .scale(3)
            .mobile(),
        )
        .column(
            Column::quantity(
                "good_milli",
                &t(lang, "manufacturing.screen.orders.made_qty"),
            )
            .scale(3),
        )
        .column(Column::date(
            "planned_start",
            &t(lang, "manufacturing.screen.orders.planned_start"),
        ))
        .column(
            Column::date(
                "planned_end",
                &t(lang, "manufacturing.screen.orders.planned_end"),
            )
            .hidden(),
        )
        .column(Column::custom(
            "source",
            &t(lang, "manufacturing.screen.orders.source"),
        ))
        .column(
            Column::select(
                "status",
                &t(lang, "manufacturing.screen.fields.status"),
                statuses,
            )
            .badges(&[
                ("planned", "info"),
                ("released", "info"),
                ("in_progress", "info"),
                ("on_hold", "warning"),
                ("completed", "success"),
                ("closed", "success"),
                ("cancelled", "info"),
                ("aborted", "error"),
            ])
            .mobile(),
        )
        .sort_by("-id")
        .row_url("/manufacturing/work-orders/{id}")
        .cards_on_mobile()
        .empty_state(
            &t(lang, "manufacturing.screen.orders.empty"),
            Some(&t(lang, "manufacturing.screen.orders.empty_text")),
        );
    let grid = super::with_item_search(grid, lang, "item_id", true);
    if can_export { grid.exports() } else { grid }
}

/// The statuses `?status=` names: `open`, or one state; none for anything else.
fn wanted(status: &str) -> Option<Vec<WorkOrderStatus>> {
    match status {
        "open" => Some(STATUSES.into_iter().filter(|s| s.is_open()).collect()),
        key => STATUSES
            .into_iter()
            .find(|s| s.as_str() == key)
            .map(|s| vec![s]),
    }
}

/// `GET /manufacturing/work-orders`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    let status = request.param("status").unwrap_or_default().to_owned();
    let visible: Vec<WorkOrder> = WorkOrder::query()
        .order_by_desc(WorkOrder::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|order| order.may_see(&user))
        .collect();
    if visible.is_empty()
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_VIEW)
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_RELEASE)
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_REPORT)
    {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for order in &visible {
            if wanted(&status).is_some_and(|set| !set.contains(&order.status)) {
                continue;
            }
            rows.push(json_row(&state, &lang, order).await?);
        }
        return Ok(Json(&rows).into_response());
    }
    let can_export = user.has_permission_anywhere(permissions::WORK_ORDERS_RELEASE)
        || user.has_permission_anywhere(permissions::WORK_ORDERS_CLOSE);
    let grid = grid(&lang, can_export);
    let mut query = WorkOrder::query()
        .where_(WorkOrder::ID.is_in(ids_or_none(visible.iter().map(|order| order.id).collect())));
    if let Some(set) = wanted(&status) {
        query = query.where_(WorkOrder::STATUS.is_in(set));
    }
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    for order in page.items() {
        extra.insert(
            order.id,
            json!({
                "item": super::work_order_page::item_text(&state, &lang, order.item_id).await,
                "source": super::work_order_page::source_text(&state, &lang, order).await?,
                "period": period_text(&lang, order.planned_start, order.planned_end),
            }),
        );
    }
    let page = page.extend(|order| extra.get(&order.id).cloned().unwrap_or_else(|| json!({})));
    let mut form = NewForm {
        today: wl_kernel::documents::today().to_string(),
        ..NewForm::default()
    };
    let mut can_create = false;
    for plant in Plant::query().order_by(Plant::ID).get(&state.db).await? {
        if plant.active && user.has_permission_in(permissions::WORK_ORDERS_CREATE, &plant.scope()) {
            can_create = true;
            form.plants.push((plant.id.to_string(), plant.name.clone()));
            for line in Line::query()
                .where_(Line::PLANT_ID.eq(plant.id))
                .order_by(Line::ID)
                .get(&state.db)
                .await?
            {
                if line.active {
                    form.lines.push((
                        line.id.to_string(),
                        format!("{} - {}, {}", line.code, line.name, plant.name),
                    ));
                }
            }
        }
    }
    form.plant_selected = form.plants.first().map(|p| p.0.clone()).unwrap_or_default();
    form.line_selected = form.lines.first().map(|l| l.0.clone()).unwrap_or_default();
    let page = WorkOrdersPage {
        tx: screen_texts(&lang),
        rows: page,
        all_url: URL.to_owned(),
        open_url: format!("{URL}?status=open"),
        open_now: status == "open",
        can_create,
        form,
    };
    Ok(View::typed(page).into_response())
}

/// A work order as the JSON answer has it.
async fn json_row(state: &AppState, lang: &Lang, order: &WorkOrder) -> Result<Row> {
    let line = match order.line_id {
        Some(line) => Line::find(&state.db, line)
            .await?
            .map_or_else(|| t(lang, "manufacturing.screen.not_set"), |line| line.code),
        None => t(lang, "manufacturing.screen.no_line"),
    };
    Ok(Row {
        id: order.id,
        url: super::work_order_page::url(order.id),
        number: order.number.clone(),
        item: super::work_order_page::item_text(state, lang, order.item_id).await,
        line,
        quantity: quantity_text(order.quantity_milli),
        produced: quantity_text(order.good_milli),
        period: period_text(lang, order.planned_start, order.planned_end),
        source: super::work_order_page::source_text(state, lang, order).await?,
        status: word(lang, "status", order.status.as_str()),
        status_kind: status_kind(order.status),
    })
}
