//! The export dispatches the evidence rule follows (`GET
//! /manufacturing/exports`, BL-MFG-12): per dispatch the invoices that bill it
//! at the zero rate, the days since it left, where it stands (*waiting*,
//! *warned* at day 60, *5% VAT due* at day 90, *evidence received*), a link to
//! attach the exit evidence to the dispatch and the customer's contract note
//! the accountant reads when the zero rate is lost. The page is a
//! `renox::grid` searched by dispatch and invoice and filtered by state and
//! day; oldest first, so the dispatch with the most days (the evidence is due
//! within a window) leads. JSON keeps answering the rows in words.

use std::collections::{BTreeMap, HashMap};

use renox::axum::http::HeaderMap;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::inventory::DISPATCH_SUBJECT_TYPE;

use super::{Texts, date_text, done, grids, party_text};
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

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/exports.html")]
pub(crate) struct ExportsPage {
    tx: Texts,
    watches: GridPage<ExportWatch>,
    /// The customers of the page, `[party id, note]`, for the sheets that
    /// change their contract note.
    notes: Vec<(i64, String)>,
}

/// The states of a watch, in the order a dispatch goes through them.
const STATES: [&str; 5] = ["waiting", "warned", "vat_due", "evidence", "evidence_late"];

/// Where a watch stands, as SQL ([`state_of`] in words).
const STATE_SQL: &str = "CASE WHEN {T}.\"evidence_on\" IS NOT NULL AND {T}.\"flagged_on\" IS NOT NULL \
    THEN 'evidence_late' WHEN {T}.\"evidence_on\" IS NOT NULL THEN 'evidence' \
    WHEN {T}.\"flagged_on\" IS NOT NULL THEN 'vat_due' \
    WHEN {T}.\"warned_on\" IS NOT NULL THEN 'warned' ELSE 'waiting' END";

/// The export evidence grid: the dispatch first and frozen, the customer,
/// the day it left and the days since, the invoices, where it stands in
/// words, the evidence link and the customer's contract note; oldest first.
pub(crate) fn grid(lang: &Lang) -> Grid {
    let text = |key: &str| t(lang, &format!("manufacturing.screen.{key}"));
    let states: Vec<(String, String)> = STATES
        .iter()
        .map(|state| ((*state).to_owned(), word(lang, "export_state", state)))
        .collect();
    grids::baseline(Grid::new("manufacturing-exports"))
        .title(&text("exports.title"))
        .column(
            Column::text("number", &text("exports.dispatch"))
                .frozen()
                .mobile()
                .searchable()
                .link("/dispatches/{dispatch_id}"),
        )
        .column(Column::custom("customer", &text("grid.customer")).mobile())
        .column(Column::date("dispatched_on", &text("exports.left")))
        .column(Column::custom("days", &text("exports.days")).mobile())
        .column(Column::text("invoices", &text("grid.invoices")).searchable())
        .column(
            Column::select("state", &text("grid.status"), states)
                .computed(STATE_SQL)
                .badges(&[
                    ("waiting", "info"),
                    ("warned", "warning"),
                    ("vat_due", "danger"),
                    ("evidence", "success"),
                    ("evidence_late", "warning"),
                ])
                .mobile(),
        )
        .column(Column::custom("evidence", &text("exports.evidence")))
        .column(Column::custom("note", &text("exports.note")))
        .sort_by("dispatched_on")
        .empty_state(&text("exports.empty"), Some(&text("exports.empty_text")))
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
            rows.push(row(&state, &lang, &watch).await?);
        }
        return Ok(Json(&rows).into_response());
    }
    // The accountant who follows the exports may export the list; the
    // customer and the note are custom columns and never leave in a file.
    let grid = grids::exporting(grid(&lang), true);
    if let Some(file) = grids::export_file(&grid, true, ExportWatch::query(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(ExportWatch::query(), &request).await?;
    let mut extra: HashMap<i64, Value> = HashMap::new();
    let mut notes: BTreeMap<i64, String> = BTreeMap::new();
    for watch in page.items() {
        let found = row(&state, &lang, watch).await?;
        if let Some(party) = watch.customer_id {
            notes.insert(party, found.note.clone());
        }
        extra.insert(
            watch.id,
            json!({
                "customer": found.customer,
                "days": found.days,
                "evidence_url": found.evidence_url,
                "note": found.note,
                "party_id": watch.customer_id,
            }),
        );
    }
    let watches = page.extend(|watch| extra.get(&watch.id).cloned().unwrap_or(Value::Null));
    Ok(View::typed(ExportsPage {
        tx: screen_texts(&lang),
        watches,
        notes: notes.into_iter().collect(),
    })
    .into_response())
}

/// A watch in words, for the JSON answer and the grid's custom cells.
async fn row(state: &AppState, lang: &Lang, watch: &ExportWatch) -> Result<Row> {
    let (key, kind) = state_of(watch);
    let note = match watch.customer_id {
        Some(party) => ContractNote::query()
            .where_(ContractNote::PARTY_ID.eq(party))
            .first(&state.db)
            .await?
            .map(|found| found.note)
            .unwrap_or_default(),
        None => String::new(),
    };
    Ok(Row {
        dispatch: watch.number.clone(),
        dispatch_url: format!("/dispatches/{}", watch.dispatch_id),
        evidence_url: format!(
            "/documents/of/{DISPATCH_SUBJECT_TYPE}/{}",
            watch.dispatch_id
        ),
        customer: party_text(
            state,
            lang,
            watch.customer_id,
            &t(lang, "manufacturing.screen.recall.unknown_customer"),
        )
        .await,
        left: date_text(watch.dispatched_on),
        days: (today() - watch.dispatched_on).num_days(),
        invoices: watch.invoices.clone(),
        state: word(lang, "export_state", key),
        state_kind: kind,
        note,
        party_id: watch.customer_id.unwrap_or_default(),
    })
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
