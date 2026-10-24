//! The export dispatches the evidence rule follows (`GET
//! /manufacturing/exports`, BL-MFG-12): per dispatch the invoices that bill it
//! at the zero rate, the days since it left, where it stands (*waiting*,
//! *warned* at day 60, *5% VAT due* at day 90, *evidence received*), a link to
//! attach the exit evidence to the dispatch and the customer's contract note
//! the accountant reads when the zero rate is lost.
//!
//! The list is the standard grid (search on the dispatch number and the
//! invoices; filters by state and date; sort by the day it left, the oldest
//! first being the most days). The state is worked out in the query so it can
//! be filtered and sorted; the customer, the days, the evidence link and the
//! contract note are custom cells. The dispatch is a link only for who may open
//! dispatches.

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::inventory::DISPATCH_SUBJECT_TYPE;

use super::{Texts, date_text, done, exported, may_open_dispatch, party_text};
use crate::exports::{self, ContractNote, ExportWatch};
use crate::permissions;
use crate::texts::{screen_texts, t, word};

/// The address of the page.
pub(crate) const URL: &str = "/manufacturing/exports";

/// One dispatch under watch.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Row {
    dispatch: String,
    dispatch_url: String,
    evidence_url: String,
    customer: String,
    left: String,
    days: i64,
    invoices: String,
    state: String,
    state_kind: &'static str,
    note: String,
    party_id: i64,
}

/// A contract note sheet of the page.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Sheet {
    id: String,
    party_id: i64,
    note: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/exports.html")]
pub(crate) struct ExportsPage {
    tx: Texts,
    rows: GridPage<ExportWatch>,
    sheets: Vec<Sheet>,
}

/// The states of a watch.
const STATES: [&str; 5] = ["waiting", "warned", "vat_due", "evidence", "evidence_late"];

/// The state of a watch as the query works it out (the same as [`state_of`]).
const STATE_SQL: &str = "CASE \
    WHEN {T}.\"evidence_on\" IS NOT NULL AND {T}.\"flagged_on\" IS NOT NULL THEN 'evidence_late' \
    WHEN {T}.\"evidence_on\" IS NOT NULL THEN 'evidence' \
    WHEN {T}.\"flagged_on\" IS NOT NULL THEN 'vat_due' \
    WHEN {T}.\"warned_on\" IS NOT NULL THEN 'warned' \
    ELSE 'waiting' END";

/// The export watches grid. The dispatch is a custom cell, a link only for
/// who may open dispatches; its number is a searchable hidden column.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let tx = |key: &str| t(lang, &format!("manufacturing.screen.exports.{key}"));
    Grid::new("manufacturing-exports")
        .title(&tx("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("dispatch", &tx("dispatch"))
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("number", &tx("dispatch"))
                .hidden()
                .searchable()
                .copyable(),
        )
        .column(Column::custom("customer", &tx("customer")).mobile())
        .column(Column::date("dispatched_on", &tx("left")).mobile())
        .column(Column::custom("days", &tx("days")).mobile())
        .column(Column::text("invoices", &tx("invoices")).searchable())
        .column(
            Column::select(
                "state",
                &tx("state"),
                STATES
                    .iter()
                    .map(|state| (*state, word(lang, "export_state", state)))
                    .collect::<Vec<_>>(),
            )
            .computed(STATE_SQL)
            .badges(&[
                ("waiting", "info"),
                ("warned", "warning"),
                ("vat_due", "error"),
                ("evidence", "success"),
                ("evidence_late", "warning"),
            ])
            .mobile(),
        )
        .column(Column::custom("evidence", &tx("evidence")))
        .column(Column::custom("note", &tx("note")))
        .sort_by("dispatched_on")
        .cards_on_mobile()
        .exports()
        .empty_state(&tx("empty"), Some(&tx("empty_text")))
}

/// The state of a watch in words and tone.
fn state_of(watch: &ExportWatch) -> (&'static str, &'static str) {
    match (watch.evidence_on, watch.flagged_on, watch.warned_on) {
        (Some(_), Some(_), _) => ("evidence_late", "warning"),
        (Some(_), None, _) => ("evidence", "success"),
        (None, Some(_), _) => ("vat_due", "error"),
        (None, None, Some(_)) => ("warned", "warning"),
        (None, None, None) => ("waiting", "info"),
    }
}

/// The contract note of a customer, empty for none.
async fn note_of(state: &AppState, customer: Option<i64>) -> Result<String> {
    Ok(match customer {
        Some(party) => ContractNote::query()
            .where_(ContractNote::PARTY_ID.eq(party))
            .first(&state.db)
            .await?
            .map(|found| found.note)
            .unwrap_or_default(),
        None => String::new(),
    })
}

/// The customer of a watch in words.
async fn customer_text(state: &AppState, lang: &Lang, watch: &ExportWatch) -> String {
    party_text(
        state,
        lang,
        watch.customer_id,
        &t(lang, "manufacturing.screen.recall.unknown_customer"),
    )
    .await
}

/// `GET /manufacturing/exports`.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    request: GridRequest,
) -> Result<Response> {
    if !user.has_permission(permissions::EXPORTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let mut rows = Vec::new();
        for watch in ExportWatch::query()
            .order_by_desc(ExportWatch::ID)
            .get(&state.db)
            .await?
        {
            let (key, kind) = state_of(&watch);
            rows.push(Row {
                dispatch: watch.number.clone(),
                dispatch_url: format!("/dispatches/{}", watch.dispatch_id),
                evidence_url: evidence_url(watch.dispatch_id),
                customer: customer_text(&state, &lang, &watch).await,
                left: date_text(watch.dispatched_on),
                days: (today() - watch.dispatched_on).num_days(),
                invoices: watch.invoices.clone(),
                state: word(&lang, "export_state", key),
                state_kind: kind,
                note: note_of(&state, watch.customer_id).await?,
                party_id: watch.customer_id.unwrap_or_default(),
            });
        }
        return Ok(Json(&rows).into_response());
    }
    let grid = grid(&lang);
    let link_dispatch = may_open_dispatch(&user);
    let query = ExportWatch::query();
    if let Some(file) = exported(&grid, &query, &request, true).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut extra = std::collections::HashMap::new();
    let mut sheets = Vec::new();
    for watch in page.items() {
        let party_id = watch.customer_id.unwrap_or_default();
        let note = note_of(&state, watch.customer_id).await?;
        let id = format!("{party_id}-{}", watch.number);
        sheets.push(Sheet {
            id: id.clone(),
            party_id,
            note: note.clone(),
        });
        extra.insert(
            watch.id,
            json!({
                "dispatch_url": if link_dispatch { format!("/dispatches/{}", watch.dispatch_id) } else { String::new() },
                "customer": customer_text(&state, &lang, watch).await,
                "days": (today() - watch.dispatched_on).num_days(),
                "evidence_url": evidence_url(watch.dispatch_id),
                "note": note,
                "sheet": id,
            }),
        );
    }
    let page = page.extend(|watch| extra.get(&watch.id).cloned().unwrap_or_else(|| json!({})));
    Ok(View::typed(ExportsPage {
        tx: screen_texts(&lang),
        rows: page,
        sheets,
    })
    .into_response())
}

/// Where the evidence of a dispatch is attached.
fn evidence_url(dispatch_id: i64) -> String {
    format!("/documents/of/{DISPATCH_SUBJECT_TYPE}/{dispatch_id}")
}

/// `POST /manufacturing/exports/notes`: the contract note of a customer.
pub(crate) async fn note(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<std::collections::BTreeMap<String, String>>,
) -> Result<Response> {
    let party = form
        .get("party_id")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .ok_or_else(|| crate::db::refused(t(&lang, "manufacturing.bad_input")))?;
    let text = form.get("note").cloned().unwrap_or_default();
    exports::set_note(&state, &user, party, &text).await?;
    Ok(done(&lang, URL, "manufacturing.screen.exports.note_done"))
}
