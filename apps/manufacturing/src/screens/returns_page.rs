//! The goods that came back and what sales decides about them (`GET
//! /manufacturing/returns`, BL-MFG-07, `MFG.RUL.10`): per order line the
//! quantity refused or returned, the reason, what is still open, the sales
//! person it is given to and the actions *New dispatch* (a draft dispatch for
//! the open quantity) and *Cancel the line* (on the order). Closed decisions
//! show how they ended. The list is the standard grid (search on the order,
//! the dispatch, the reason and the sales person; filters by status, reason
//! and date); `?status=open` (the link in the menu) or `?status=closed` still
//! narrows the query. The item, the customer and the buttons are custom cells.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;

use super::{
    Act, Texts, done, exported, may_open_dispatch, parse_quantity, party_text, quantity_text,
    user_text,
};
use crate::permissions;
use crate::returns::{self, ReturnDecision};
use crate::texts::{screen_texts, t, tf, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/returns";

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

/// The sheet of a row's *New dispatch* button.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Sheet {
    id: i64,
    open: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/returns.html")]
pub(crate) struct ReturnsPage {
    tx: Texts,
    rows: GridPage<ReturnDecision>,
    all_url: String,
    open_url: String,
    closed_url: String,
    current: String,
    sheets: Vec<Sheet>,
}

/// The decisions grid; `can_export` adds the export menu. The order and the
/// dispatch are custom cells (links to other modules' pages, the dispatch only
/// for who may open it); their numbers are searchable hidden columns.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.returns.{key}"));
    let grid = Grid::new("manufacturing-returns")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(Column::custom("order", &tx("order")).frozen().mobile())
        .column(
            Column::text("order_number", &tx("order_number"))
                .hidden()
                .searchable(),
        )
        .column(Column::number("line_id", &tx("line")).hidden())
        .column(Column::custom("item", &t(lang, "manufacturing.screen.fact.item")).mobile())
        .column(Column::custom("customer", &tx("customer")))
        .column(
            Column::quantity("refused_milli", &tx("refused"))
                .scale(3)
                .mobile(),
        )
        .column(
            Column::quantity("open_milli", &tx("open"))
                .scale(3)
                .mobile(),
        )
        .column(Column::text("reason", &tx("reason")).searchable())
        .column(Column::custom("dispatch", &tx("dispatch_number")))
        .column(
            Column::text("dispatch_number", &tx("dispatch_number"))
                .hidden()
                .searchable(),
        )
        .column(Column::related("owner", &tx("owner"), "users", "owner_id", "name").searchable())
        .column(
            Column::select(
                "status",
                &t(lang, "manufacturing.screen.fields.status"),
                [
                    ("open", word(lang, "return_status", "open")),
                    ("closed", word(lang, "return_status", "closed")),
                ],
            )
            .badges(&[("open", "warning"), ("closed", "success")])
            .mobile(),
        )
        .column(
            Column::select(
                "closed_why",
                &tx("closed_why"),
                ["delivered_again", "cancelled", "order_cancelled"]
                    .into_iter()
                    .map(|why| (why, word(lang, "return_closed", why)))
                    .collect::<Vec<_>>(),
            )
            .hidden(),
        )
        .column(Column::datetime("created_at", &tx("recorded")).hidden())
        .column(Column::custom("actions", &tx("actions")).frozen_right())
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&tx("empty"), Some(&tx("empty_text")));
    let grid = super::with_item_search(grid, lang, "item_id", true);
    if can_export { grid.exports() } else { grid }
}

/// `GET /manufacturing/returns`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
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
    let status = request.param("status").unwrap_or_default().to_owned();
    let act_of = |found: &ReturnDecision| {
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
            if !status.is_empty() && found.status != status {
                continue;
            }
            let open = found.is_open();
            let (new_dispatch, cancel_line) = act_of(&found);
            rows.push(Row {
                id: found.id,
                order: found.order_number.clone(),
                order_url: format!("/sales-orders/{}", found.order_id),
                line: tf(
                    &lang,
                    "manufacturing.screen.returns.line_name",
                    &[("id", &found.line_id)],
                ),
                item: super::names::item_text(&state, &lang, found.item_id).await,
                dispatch: found.dispatch_number.clone(),
                dispatch_url: format!("/dispatches/{}", found.dispatch_id),
                customer: customer_text(&state, &lang, &found).await,
                refused: quantity_text(found.refused_milli),
                open: quantity_text(found.open_milli),
                reason: found.reason.clone().unwrap_or_default(),
                owner: user_text(
                    &state,
                    found.owner_id,
                    &t(&lang, "manufacturing.screen.not_set"),
                )
                .await,
                status: word(&lang, "return_status", &found.status),
                status_kind: if open { "warning" } else { "success" },
                closed: found
                    .closed_why
                    .as_deref()
                    .map(|why| word(&lang, "return_closed", why))
                    .unwrap_or_default(),
                new_dispatch,
                cancel_line,
                task_url: found
                    .task_id
                    .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
                    .unwrap_or_default(),
            });
        }
        return Ok(Json(&rows).into_response());
    }
    let can_export = may_decide;
    let grid = grid(&lang, can_export);
    let link_dispatch = may_open_dispatch(&user);
    let mut query = ReturnDecision::query();
    if !status.is_empty() {
        query = query.where_(ReturnDecision::STATUS.eq(status.clone()));
    }
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    let mut sheets = Vec::new();
    for found in page.items() {
        let (new_dispatch, cancel_line) = act_of(found);
        let ready = new_dispatch.shown && new_dispatch.blocked.is_empty();
        if ready {
            sheets.push(Sheet {
                id: found.id,
                open: quantity_text(found.open_milli),
            });
        }
        extra.insert(
            found.id,
            json!({
                "order_url": format!("/sales-orders/{}", found.order_id),
                "dispatch_url": if link_dispatch { format!("/dispatches/{}", found.dispatch_id) } else { String::new() },
                "item": super::names::item_text(&state, &lang, found.item_id).await,
                "customer": customer_text(&state, &lang, found).await,
                "closed": found.closed_why.as_deref().map(|why| word(&lang, "return_closed", why)).unwrap_or_default(),
                "can_dispatch": ready,
                "dispatch_blocked": if new_dispatch.shown { new_dispatch.blocked } else { String::new() },
                "can_cancel": cancel_line.shown,
            }),
        );
    }
    let page = page.extend(|found| extra.get(&found.id).cloned().unwrap_or_else(|| json!({})));
    let current = match status.as_str() {
        "open" => "open",
        "closed" => "closed",
        _ => "",
    };
    Ok(View::typed(ReturnsPage {
        tx: screen_texts(&lang),
        rows: page,
        all_url: URL.to_owned(),
        open_url: format!("{URL}?status=open"),
        closed_url: format!("{URL}?status=closed"),
        current: current.to_owned(),
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
