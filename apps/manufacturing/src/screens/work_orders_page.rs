//! The work order list (`GET /manufacturing/work-orders`, `docs/UX.md` §2.1)
//! and its *New work order* sheet.
//!
//! The list shows the work orders of the plants and lines the person works
//! for: the number, the item, the line, the quantity made so far, the status
//! and the source (a sales order or stock). The status filter is
//! `?status=<key>` (`open` is every state before `completed`).

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, period_text, quantity_text};
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
    rows: Vec<Row>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
    can_create: bool,
    form: NewForm,
}

/// The status tabs: all, the open ones and each state.
fn tabs(lang: &Lang) -> Vec<(String, String)> {
    let mut tabs = vec![
        (URL.to_owned(), t(lang, "manufacturing.screen.filter.all")),
        (
            format!("{URL}?status=open"),
            t(lang, "manufacturing.screen.filter.open"),
        ),
    ];
    for status in ["in_progress", "on_hold", "completed", "aborted"] {
        tabs.push((
            format!("{URL}?status={status}"),
            word(lang, "status", status),
        ));
    }
    tabs
}

/// `GET /manufacturing/work-orders`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
) -> Result<Response> {
    let mut rows = Vec::new();
    let mut any = false;
    for order in WorkOrder::query()
        .order_by_desc(WorkOrder::ID)
        .get(&state.db)
        .await?
    {
        if !order.may_see(&user) {
            continue;
        }
        any = true;
        let wanted = match filter.status.as_str() {
            "" => true,
            "open" => order.status.is_open(),
            key => order.status.as_str() == key,
        };
        if !wanted {
            continue;
        }
        let line = match order.line_id {
            Some(line) => Line::find(&state.db, line).await?.map_or_else(
                || t(&lang, "manufacturing.screen.not_set"),
                |line| line.code,
            ),
            None => t(&lang, "manufacturing.screen.no_line"),
        };
        rows.push(Row {
            id: order.id,
            url: super::work_order_page::url(order.id),
            number: order.number.clone(),
            item: super::work_order_page::item_text(&lang, order.item_id),
            line,
            quantity: quantity_text(order.quantity_milli),
            produced: quantity_text(order.good_milli),
            period: period_text(&lang, order.planned_start, order.planned_end),
            source: super::work_order_page::source_text(&state, &lang, &order).await?,
            status: word(&lang, "status", order.status.as_str()),
            status_kind: status_kind(order.status),
        });
    }
    if !any
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_VIEW)
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_RELEASE)
        && !user.has_permission_anywhere(permissions::WORK_ORDERS_REPORT)
    {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
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
        rows,
        status_tabs: tabs(&lang),
        status_current: match filter.status.as_str() {
            "" => URL.to_owned(),
            key => format!("{URL}?status={key}"),
        },
        can_create,
        form,
    };
    Ok(View::typed(page).into_response())
}
