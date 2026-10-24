//! The recalls as the recall owner reads them in 07 (`GET
//! /manufacturing/recalls` and `/manufacturing/recalls/{id}`, BL-MFG-10): per
//! customer, the work orders and the dispatches that got a recalled lot, each
//! *shipped* or *not shipped*, the dispatches to bring back and the ones
//! flagged for a re-pick. The recall itself (opening it, the holds, the
//! notices, closing it) is wl-inventory's page at `/recalls/{id}`.
//!
//! The list is the standard grid over wl-inventory's recalls table, read only
//! ([`RecallRow`]; the vertical has no table of its own for them, and the
//! trace below still comes through the kernel contract): search on the number
//! and the reason, filters by status, source and date.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{DISPATCH_SUBJECT_TYPE, RECALL_SUBJECT_TYPE, lot_trace};

use super::{Texts, date_text, exported, party_text, quantity_text};
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

/// A recall of wl-inventory's table, as the list reads it (never written
/// here).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_recalls")]
pub struct RecallRow {
    /// The primary key.
    pub id: i64,
    /// The number, `RCL-2026-0001`.
    pub number: Option<String>,
    /// `supplier`, `authority`, `internal` or `complaint`.
    pub source: String,
    /// Why it was opened.
    pub reason: String,
    /// `open`, `traced` or `closed`.
    pub status: String,
    /// When it was opened.
    pub opened_at: Option<renox::db::DateTime>,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/recalls.html")]
pub(crate) struct RecallsPage {
    tx: Texts,
    rows: GridPage<RecallRow>,
}

/// The recalls grid.
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.recalls.{key}"));
    let grid = Grid::new("manufacturing-recalls")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &tx("recall"))
                .frozen()
                .mobile()
                .searchable()
                .copyable()
                .link("/manufacturing/recalls/{id}"),
        )
        .column(Column::text("reason", &tx("reason")).mobile().searchable())
        .column(Column::select(
            "source",
            &tx("source"),
            ["supplier", "authority", "internal", "complaint"]
                .into_iter()
                .map(|source| (source, word(lang, "recall_source", source)))
                .collect::<Vec<_>>(),
        ))
        .column(
            Column::select(
                "status",
                &t(lang, "manufacturing.screen.fields.status"),
                ["open", "traced", "closed"]
                    .into_iter()
                    .map(|status| (status, word(lang, "recall_status", status)))
                    .collect::<Vec<_>>(),
            )
            .badges(&[
                ("open", "error"),
                ("traced", "warning"),
                ("closed", "success"),
            ])
            .mobile(),
        )
        .column(Column::datetime("opened_at", &tx("opened")).mobile())
        .sort_by("-id")
        .row_url("/manufacturing/recalls/{id}")
        .cards_on_mobile()
        .empty_state(&tx("empty"), Some(&tx("empty_text")));
    if can_export { grid.exports() } else { grid }
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
    request: GridRequest,
) -> Result<Response> {
    if !may_read(&user) {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
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
        return Ok(Json(&rows).into_response());
    }
    let can_export = user.has_permission_anywhere(permissions::RECALLS_MANAGE);
    let grid = grid(&lang, can_export);
    let query = RecallRow::query();
    if let Some(file) = exported(&grid, &query, &request, can_export).await? {
        return Ok(file);
    }
    let rows = grid.page(query, &request).await?;
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
