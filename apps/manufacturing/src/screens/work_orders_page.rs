//! The work order list (`GET /manufacturing/work-orders`, `docs/UX.md` §2.1)
//! and its *New work order* sheet.
//!
//! The list is a `renox::grid` of the work orders of the plants and lines the
//! person works for: the number, the item, the status, the line, the quantity
//! made and planned, the planned days and the source (a sales order or
//! stock); searched by number and line, filtered and sorted by any column.
//! A `?status=<key>` link (the dashboard tiles; `open` is every state before
//! `completed`) narrows the rows to those states under the grid's own
//! filters; JSON keeps answering the rows in words, filtered the same way.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};

use super::{Texts, grids, period_text, quantity_text};
use crate::permissions;
use crate::plant::{Line, Plant};
use crate::texts::{screen_texts, t, word};
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// The address of the list.
pub(crate) const URL: &str = "/manufacturing/work-orders";

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

/// The list's filter.
#[derive(Deserialize, Default)]
pub(crate) struct ListFilter {
    /// A status key, or `open`.
    #[serde(default)]
    status: String,
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
    orders: GridPage<WorkOrder>,
    can_create: bool,
    form: NewForm,
}

/// The work orders grid: the number first and frozen, the item, the status in
/// words, the line, the quantity made and planned, the planned days and the
/// source; newest first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    let statuses: Vec<(String, String)> = WorkOrderStatus::ALL
        .iter()
        .map(|status| (status.as_str().to_owned(), word(lang, "status", status.as_str())))
        .collect();
    let tones: Vec<(&str, &str)> = WorkOrderStatus::ALL
        .iter()
        .map(|status| (status.as_str(), grids::tone(status_kind(*status))))
        .collect();
    grids::baseline(Grid::new("manufacturing-work-orders"))
        .title(&text("orders.title"))
        .column(
            Column::text("number", &text("orders.order"))
                .frozen()
                .mobile()
                .searchable()
                .link(&format!("{URL}/{{id}}")),
        )
        .column(Column::custom("item", &text("grid.item")).mobile())
        .column(
            Column::select("status", &text("grid.status"), statuses)
                .badges(&tones)
                .mobile(),
        )
        .column(
            Column::related(
                "line",
                &text("orders.line"),
                "manufacturing_lines",
                "line_id",
                "code",
            )
            .searchable(),
        )
        .column(
            Column::quantity("good_milli", &text("grid.made"))
                .scale(3)
                .mobile(),
        )
        .column(Column::quantity("quantity_milli", &text("grid.planned_quantity")).scale(3))
        .column(Column::date("planned_start", &text("grid.planned_start")))
        .column(Column::date("planned_end", &text("grid.planned_end")).hidden())
        .column(Column::custom("source", &text("grid.source")))
        .sort_by("-id")
        .row_url(&format!("{URL}/{{id}}"))
        .empty_state(&text("orders.empty"), Some(&text("orders.empty_text")))
}

/// The states a `?status=<key>` link names: `open` is every state
/// before `completed`; nothing for no filter.
fn linked_statuses(key: &str) -> Vec<&'static str> {
    match key {
        "" => Vec::new(),
        "open" => WorkOrderStatus::ALL
            .iter()
            .filter(|status| status.is_open())
            .map(|status| status.as_str())
            .collect(),
        key => WorkOrderStatus::ALL
            .iter()
            .map(|status| status.as_str())
            .filter(|status| *status == key)
            .collect(),
    }
}

/// `GET /manufacturing/work-orders`: the grid for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
    request: GridRequest,
) -> Result<Response> {
    let mut visible = Vec::new();
    for order in WorkOrder::query()
        .order_by_desc(WorkOrder::ID)
        .get(&state.db)
        .await?
    {
        if order.may_see(&user) {
            visible.push(order);
        }
    }
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
            let wanted = match filter.status.as_str() {
                "" => true,
                "open" => order.status.is_open(),
                key => order.status.as_str() == key,
            };
            if wanted {
                rows.push(row(&state, &lang, order).await?);
            }
        }
        return Ok(Json(&rows).into_response());
    }
    // The rows carry no cost; the people who plan and release the work
    // orders may export the list, not the floor (operators, storekeepers).
    let can_export = user.has_permission_anywhere(permissions::WORK_ORDERS_CREATE)
        || user.has_permission_anywhere(permissions::WORK_ORDERS_RELEASE);
    let grid = grids::exporting(grid(&lang), can_export);
    let mut query = WorkOrder::query().where_in("id", visible.iter().map(|order| order.id));
    if !filter.status.is_empty() {
        query = query.where_in("status", linked_statuses(&filter.status));
    }
    if let Some(file) = grids::export_file(&grid, can_export, query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    for order in page.items() {
        extra.insert(
            order.id,
            json!({
                "item": super::work_order_page::item_text(&state, &lang, order.item_id).await,
                "source": super::work_order_page::source_text(&state, &lang, order).await?,
            }),
        );
    }
    let orders = page.extend(|order| extra.get(&order.id).cloned().unwrap_or(Value::Null));
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
        orders,
        can_create,
        form,
    };
    Ok(View::typed(page).into_response())
}

/// A work order in words, for the JSON answer.
async fn row(state: &AppState, lang: &Lang, order: &WorkOrder) -> Result<Row> {
    let line = match order.line_id {
        Some(line) => Line::find(&state.db, line).await?.map_or_else(
            || t(lang, "manufacturing.screen.not_set"),
            |line| line.code,
        ),
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
