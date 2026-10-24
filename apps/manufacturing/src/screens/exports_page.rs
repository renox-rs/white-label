//! The export dispatches the evidence rule follows (`GET
//! /manufacturing/exports`, BL-MFG-12): per dispatch the invoices that bill it
//! at the zero rate, the days since it left, where it stands (*waiting*,
//! *warned* at day 60, *5% VAT due* at day 90, *evidence received*), a link to
//! attach the exit evidence to the dispatch and the customer's contract note
//! the accountant reads when the zero rate is lost.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::inventory::DISPATCH_SUBJECT_TYPE;

use super::{Texts, date_text, done, party_text};
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
    rows: Vec<Row>,
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
) -> Result<Response> {
    if !user.has_permission(permissions::EXPORTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut rows = Vec::new();
    for watch in ExportWatch::query()
        .order_by_desc(ExportWatch::ID)
        .get(&state.db)
        .await?
    {
        let (key, kind) = state_of(&watch);
        let note = match watch.customer_id {
            Some(party) => ContractNote::query()
                .where_(ContractNote::PARTY_ID.eq(party))
                .first(&state.db)
                .await?
                .map(|found| found.note)
                .unwrap_or_default(),
            None => String::new(),
        };
        rows.push(Row {
            dispatch: watch.number.clone(),
            dispatch_url: format!("/dispatches/{}", watch.dispatch_id),
            evidence_url: format!(
                "/documents/of/{DISPATCH_SUBJECT_TYPE}/{}",
                watch.dispatch_id
            ),
            customer: party_text(
                &state,
                &lang,
                watch.customer_id,
                &t(&lang, "manufacturing.screen.recall.unknown_customer"),
            )
            .await,
            left: date_text(watch.dispatched_on),
            days: (today() - watch.dispatched_on).num_days(),
            invoices: watch.invoices.clone(),
            state: word(&lang, "export_state", key),
            state_kind: kind,
            note,
            party_id: watch.customer_id.unwrap_or_default(),
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        return Ok(Json(&rows).into_response());
    }
    Ok(View::typed(ExportsPage {
        tx: screen_texts(&lang),
        rows,
    })
    .into_response())
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
