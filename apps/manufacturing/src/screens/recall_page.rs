//! The recalls as the recall owner reads them in 07 (`GET
//! /manufacturing/recalls` and `/manufacturing/recalls/{id}`, BL-MFG-10): per
//! customer, the work orders and the dispatches that got a recalled lot, each
//! *shipped* or *not shipped*, the dispatches to bring back and the ones
//! flagged for a re-pick. The recall itself (opening it, the holds, the
//! notices, closing it) is wl-inventory's page at `/recalls/{id}`.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::inventory::{DISPATCH_SUBJECT_TYPE, RECALL_SUBJECT_TYPE, lot_trace};

use super::{Texts, date_text, party_text, quantity_text};
use crate::permissions;
use crate::recalls::{self, TraceRow};
use crate::texts::{screen_texts, t, tf, word};

/// The address of the list.
pub(crate) const URL: &str = "/manufacturing/recalls";

/// The address of recall `id`.
pub(crate) fn url(id: i64) -> String {
    format!("{URL}/{id}")
}

/// The address of the page of a dispatch or a work order.
fn record_url(subject: &Subject) -> String {
    if subject.subject_type == DISPATCH_SUBJECT_TYPE {
        format!("/dispatches/{}", subject.subject_id)
    } else {
        super::work_order_page::url(subject.subject_id)
    }
}

/// Whether `user` may read the recalls.
fn may_read(user: &AuthUser) -> bool {
    user.has_permission(permissions::RECALLS_VIEW)
        || user.has_permission(permissions::RECALLS_MANAGE)
        || user.has_permission_anywhere(permissions::RECALLS_VIEW)
        || user.has_permission_anywhere(permissions::RECALLS_MANAGE)
}

/// A recall of the list.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    url: String,
    number: String,
    reason: String,
    opened: String,
    status: String,
    status_kind: &'static str,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/recalls.html")]
pub(crate) struct RecallsPage {
    tx: Texts,
    rows: Vec<Row>,
}

fn status_kind(status: &str) -> &'static str {
    match status {
        "open" => "error",
        "traced" => "warning",
        _ => "success",
    }
}

/// `GET /manufacturing/recalls`.
pub(crate) async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    if !may_read(&user) {
        return Err(Error::Forbidden);
    }
    let rows: Vec<Row> = lot_trace(&state)
        .recalls(&state)
        .await?
        .into_iter()
        .map(|recall| Row {
            url: url(recall.recall.subject_id),
            number: recall.number,
            reason: recall.reason,
            opened: recall
                .opened_on
                .map_or_else(|| t(&lang, "manufacturing.screen.not_set"), date_text),
            status: word(&lang, "recall_status", &recall.status),
            status_kind: status_kind(&recall.status),
        })
        .collect();
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    Ok(View::typed(RecallsPage {
        tx: screen_texts(&lang),
        rows,
    })
    .into_response())
}

/// One line of the trace in words.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Line {
    customer: String,
    record: String,
    record_url: String,
    lot: String,
    quantity: String,
    status: String,
    status_kind: &'static str,
    day: String,
    notice: String,
    note: String,
}

/// What the record page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/recall.html")]
pub(crate) struct RecallPage {
    tx: Texts,
    number: String,
    subtitle: String,
    status: String,
    status_kind: &'static str,
    recall_url: String,
    summary: Vec<String>,
    lines: Vec<Line>,
    bring_back: Vec<Line>,
    repick: Vec<Line>,
}

async fn line_of(state: &AppState, lang: &Lang, row: &TraceRow) -> Line {
    let customer = party_text(
        state,
        lang,
        row.customer,
        &t(lang, "manufacturing.screen.recall.unknown_customer"),
    )
    .await;
    let note = if row.repick {
        t(lang, "manufacturing.screen.recall.repick_asked")
    } else if row.bring_back {
        t(lang, "manufacturing.screen.recall.bring_back_note")
    } else {
        String::new()
    };
    Line {
        customer,
        record: row.label.clone(),
        record_url: record_url(&row.subject),
        lot: row.lot.clone(),
        quantity: quantity_text(row.quantity_milli),
        status: word(
            lang,
            "shipping",
            if row.shipped {
                "shipped"
            } else {
                "not_shipped"
            },
        ),
        status_kind: if row.shipped { "info" } else { "warning" },
        day: row
            .on
            .map_or_else(|| t(lang, "manufacturing.screen.not_set"), date_text),
        notice: word(lang, "notice", &row.notice),
        note,
    }
}

/// `GET /manufacturing/recalls/{id}`: the page for a browser, JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !may_read(&user) {
        return Err(Error::Forbidden);
    }
    let found = lot_trace(&state)
        .recalls(&state)
        .await?
        .into_iter()
        .find(|recall| recall.recall == Subject::new(RECALL_SUBJECT_TYPE, id))
        .ok_or(Error::NotFound)?;
    let rows = recalls::trace(&state, id).await?;
    let mut lines = Vec::new();
    let mut bring_back = Vec::new();
    let mut repick = Vec::new();
    let mut per_customer: BTreeMap<Option<i64>, (usize, usize)> = BTreeMap::new();
    for row in &rows {
        let line = line_of(&state, &lang, row).await;
        let count = per_customer.entry(row.customer).or_default();
        if row.shipped {
            count.0 += 1;
        } else {
            count.1 += 1;
        }
        if row.bring_back {
            bring_back.push(line.clone());
        }
        if row.repick {
            repick.push(line.clone());
        }
        lines.push(line);
    }
    let mut summary = Vec::new();
    for (customer, (shipped, held)) in &per_customer {
        let name = party_text(
            &state,
            &lang,
            *customer,
            &t(&lang, "manufacturing.screen.recall.unknown_customer"),
        )
        .await;
        summary.push(tf(
            &lang,
            "manufacturing.screen.recall.summary",
            &[("customer", &name), ("shipped", shipped), ("held", held)],
        ));
    }
    let page = RecallPage {
        tx: screen_texts(&lang),
        number: found.number.clone(),
        subtitle: found.reason.clone(),
        status: word(&lang, "recall_status", &found.status),
        status_kind: status_kind(&found.status),
        recall_url: format!("/recalls/{id}"),
        summary,
        lines,
        bring_back,
        repick,
    };
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}
