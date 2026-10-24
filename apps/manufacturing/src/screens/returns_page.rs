//! The goods that came back and what sales decides about them (`GET
//! /manufacturing/returns`, BL-MFG-07, `MFG.RUL.10`): per order line the
//! quantity refused or returned, the reason, what is still open, the sales
//! person it is given to and the actions *New dispatch* (a draft dispatch for
//! the open quantity) and *Cancel the line* (on the order). Closed decisions
//! show how they ended. `?status=open` (the default for the link in the menu)
//! or `?status=closed` filters.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Act, Texts, done, parse_quantity, party_text, quantity_text, user_text};
use crate::permissions;
use crate::returns::{self, ReturnDecision};
use crate::texts::{screen_texts, t, tf, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/returns";

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
    rows: Vec<Row>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
}

fn tabs(lang: &Lang) -> Vec<(String, String)> {
    vec![
        (URL.to_owned(), t(lang, "manufacturing.screen.filter.all")),
        (
            format!("{URL}?status=open"),
            word(lang, "return_status", "open"),
        ),
        (
            format!("{URL}?status=closed"),
            word(lang, "return_status", "closed"),
        ),
    ]
}

/// `GET /manufacturing/returns`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<Filter>,
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
    let mut rows = Vec::new();
    for found in ReturnDecision::query()
        .order_by_desc(ReturnDecision::ID)
        .get(&state.db)
        .await?
    {
        if !filter.status.is_empty() && found.status != filter.status {
            continue;
        }
        let open = found.is_open();
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
            customer: party_text(
                &state,
                &lang,
                found.customer_id,
                &t(&lang, "manufacturing.screen.recall.unknown_customer"),
            )
            .await,
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
            new_dispatch: if open && may_decide {
                Act::ready()
            } else if may_decide {
                Act::blocked(t(&lang, "manufacturing.screen.returns.closed_reason"))
            } else {
                Act::hidden()
            },
            cancel_line: if open && may_cancel {
                Act::ready()
            } else {
                Act::hidden()
            },
            task_url: found
                .task_id
                .map(|id| wl_kernel::tickets::ticket_path(wl_kernel::tickets::TicketId(id)))
                .unwrap_or_default(),
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    let current = match filter.status.as_str() {
        "" => URL.to_owned(),
        key => format!("{URL}?status={key}"),
    };
    Ok(View::typed(ReturnsPage {
        tx: screen_texts(&lang),
        rows,
        status_tabs: tabs(&lang),
        status_current: current,
    })
    .into_response())
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
