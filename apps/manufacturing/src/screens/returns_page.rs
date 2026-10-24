//! The goods that came back and what sales decides about them (`GET
//! /manufacturing/returns`, BL-MFG-07, `MFG.RUL.10`): per order line the
//! quantity refused or returned, the reason, what is still open, the sales
//! person it is given to and the actions *New dispatch* (a draft dispatch for
//! the open quantity) and *Cancel the line* (on the order). Closed decisions
//! show how they ended. The page is a `renox::grid` searched by order,
//! dispatch and reason and filtered by status, reason and quantities; a
//! `?status=open` or `?status=closed` link narrows the rows to that status
//! under the grid's own filters. JSON keeps answering the rows in words.

use std::collections::HashMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::{Deserialize, Serialize};

use super::{Act, Texts, done, grids, parse_quantity, party_text, quantity_text, user_text};
use crate::permissions;
use crate::returns::{self, ReturnDecision};
use crate::texts::{screen_texts, t, tf, word};

/// The page's filter.
#[derive(Deserialize, Default)]
pub(crate) struct Filter {
    /// `open`, `closed`, or empty for both.
    #[serde(default)]
    status: String,
}

/// A decision of the list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    id: i64,
    order: String,
    order_url: String,
    line: String,
    item: String,
    dispatch: String,
    dispatch_url: String,
    customer: String,
    refused: String,
    open: String,
    reason: String,
    owner: String,
    status: String,
    status_kind: &'static str,
    closed: String,
    new_dispatch: Act,
    cancel_line: Act,
    task_url: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/returns.html")]
pub(crate) struct ReturnsPage {
    tx: Texts,
    decisions: GridPage<ReturnDecision>,
    /// The open decisions of the page sales may act on, `[id, open
    /// quantity]`, for the *New dispatch* sheets.
    sheets: Vec<(i64, String)>,
}

/// The returned goods grid: the order first and frozen, the item, the
/// customer, the status in words and how it ended, the quantity that came
/// back and what is still open, the reason, the dispatch, the sales person
/// and the actions; newest first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    let statuses: Vec<(String, String)> = [returns::decision::OPEN, returns::decision::CLOSED]
        .iter()
        .map(|status| ((*status).to_owned(), word(lang, "return_status", status)))
        .collect();
    let endings: Vec<(String, String)> = ["delivered_again", "cancelled", "order_cancelled"]
        .iter()
        .map(|why| ((*why).to_owned(), word(lang, "return_closed", why)))
        .collect();
    grids::baseline(Grid::new("manufacturing-returns"))
        .title(&text("returns.title"))
        .column(
            Column::text("order_number", &text("returns.order"))
                .frozen()
                .mobile()
                .searchable()
                .link("/sales-orders/{order_id}"),
        )
        .column(Column::custom("item", &text("grid.item")).mobile())
        .column(Column::custom("customer", &text("grid.customer")))
        .column(
            Column::select("status", &text("grid.status"), statuses)
                .badges(&[
                    (returns::decision::OPEN, "warning"),
                    (returns::decision::CLOSED, "success"),
                ])
                .mobile(),
        )
        .column(Column::select("closed_why", &text("grid.ended"), endings).hidden())
        .column(Column::quantity("refused_milli", &text("returns.refused")).scale(3))
        .column(
            Column::quantity("open_milli", &text("returns.open"))
                .scale(3)
                .mobile(),
        )
        .column(Column::text("reason", &text("returns.reason")).searchable())
        .column(
            Column::text("dispatch_number", &text("returns.dispatch"))
                .searchable()
                .link("/dispatches/{dispatch_id}"),
        )
        .column(Column::related(
            "owner",
            &text("returns.owner"),
            "users",
            "owner_id",
            "name",
        ))
        .column(Column::custom("actions", "").frozen_right())
        .sort_by("-id")
        .empty_state(&text("returns.empty"), Some(&text("returns.empty_text")))
}

/// `GET /manufacturing/returns`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
    request: GridRequest,
) -> Result<Response> {
    let sees = user.has_permission(permissions::RETURNED_GOODS_VIEW)
        || user.has_permission_anywhere(permissions::RETURNED_GOODS_VIEW)
        || user.has_permission(permissions::RETURNED_GOODS_DECIDE)
        || user.has_permission_anywhere(permissions::RETURNED_GOODS_DECIDE);
    if !sees {
        return Err(Error::Forbidden);
    }
    let may_decide = user.has_permission(permissions::RETURNED_GOODS_DECIDE)
        || user.has_permission_anywhere(permissions::RETURNED_GOODS_DECIDE);
    let may_cancel = user.has_permission("sales_orders.cancel")
        || user.has_permission_anywhere("sales_orders.cancel");
    let decided = |found: &ReturnDecision| -> (Act, Act) {
        let open = found.is_open();
        let new_dispatch = if open && may_decide {
            Act::ready()
        } else if may_decide {
            Act::blocked(t(&lang, "manufacturing.screen.returns.closed_reason"))
        } else {
            Act::hidden()
        };
        let cancel_line = if open && may_cancel {
            Act::ready()
        } else {
            Act::hidden()
        };
        (new_dispatch, cancel_line)
    };
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for found in ReturnDecision::query()
            .order_by_desc(ReturnDecision::ID)
            .get(&state.db)
            .await?
        {
            if !filter.status.is_empty() && found.status != filter.status {
                continue;
            }
            let (new_dispatch, cancel_line) = decided(&found);
            rows.push(row(&state, &lang, &found, new_dispatch, cancel_line).await);
        }
        return Ok(Json(&rows).into_response());
    }
    let mut query = ReturnDecision::query();
    if !filter.status.is_empty() {
        query = query.where_eq("status", filter.status.clone());
    }
    // Everyone who sees the page may export it; the customer is a custom
    // column and never leaves in a file.
    let grid = grids::exporting(grid(&lang), true);
    if let Some(file) = grids::export_file(&grid, true, query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    let mut sheets = Vec::new();
    for found in page.items() {
        let (new_dispatch, cancel_line) = decided(found);
        if new_dispatch.shown && new_dispatch.blocked.is_empty() {
            sheets.push((found.id, quantity_text(found.open_milli)));
        }
        extra.insert(
            found.id,
            json!({
                "item": super::names::item_text(&state, &lang, found.item_id).await,
                "customer": customer_text(&state, &lang, found).await,
                "new_dispatch": new_dispatch,
                "cancel_line": cancel_line,
            }),
        );
    }
    let decisions = page.extend(|found| extra.get(&found.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(ReturnsPage {
        tx: screen_texts(&lang),
        decisions,
        sheets,
    })
    .into_response())
}

/// The customer of a decision in words.
async fn customer_text(state: &AppState, lang: &Lang, found: &ReturnDecision) -> String {
    party_text(
        state,
        lang,
        found.customer_id,
        &t(lang, "manufacturing.screen.recall.unknown_customer"),
    )
    .await
}

/// A decision in words, for the JSON answer.
async fn row(
    state: &AppState,
    lang: &Lang,
    found: &ReturnDecision,
    new_dispatch: Act,
    cancel_line: Act,
) -> Row {
    let open = found.is_open();
    Row {
        id: found.id,
        order: found.order_number.clone(),
        order_url: format!("/sales-orders/{}", found.order_id),
        line: tf(
            lang,
            "manufacturing.screen.returns.line_name",
            &[("id", &found.line_id)],
        ),
        item: super::names::item_text(state, lang, found.item_id).await,
        dispatch: found.dispatch_number.clone(),
        dispatch_url: format!("/dispatches/{}", found.dispatch_id),
        customer: customer_text(state, lang, found).await,
        refused: quantity_text(found.refused_milli),
        open: quantity_text(found.open_milli),
        reason: found.reason.clone().unwrap_or_default(),
        owner: user_text(
            state,
            found.owner_id,
            &t(lang, "manufacturing.screen.not_set"),
        )
        .await,
        status: word(lang, "return_status", &found.status),
        status_kind: if open { "warning" } else { "success" },
        closed: found
            .closed_why
            .as_deref()
            .map(|why| word(lang, "return_closed", why))
            .unwrap_or_default(),
        new_dispatch,
        cancel_line,
        task_url: found
            .task_id
            .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
            .unwrap_or_default(),
    }
}

/// `POST /manufacturing/returns/{id}/dispatch`: a draft dispatch for the open
/// quantity, or for `quantity`.
pub(crate) async fn new_dispatch(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let quantity = match form
        .get("quantity")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(text) => Some(parse_quantity(text).ok_or_else(|| {
            crate::db::refused(t(&lang, "manufacturing.screen.returns.bad_quantity"))
        })?),
        None => None,
    };
    let made = returns::new_dispatch(&state, &user, id, quantity).await?;
    Ok(done(
        &lang,
        &format!("/dispatches/{}", made.dispatch.subject_id),
        "manufacturing.screen.returns.dispatch_done",
    ))
}
