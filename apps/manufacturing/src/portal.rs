//! The customer buyer's pages in the portal (BL-MFG-13, #394, 07 §4.5 step 5,
//! `docs/UX.md` §2.10): the open invoices of the companies the signed-in
//! person acts for, and the form to dispute one.
//!
//! - **Who.** A person who acts for no party gets a 403 (staff use the
//!   back-office pages); an invoice of another company answers 404.
//! - **The form.** A reason (quantity, price, quality, other) and the buyer's
//!   text; for *quality* the lines of the invoice to tick (all lines when none
//!   is ticked, and no line selection is offered for another reason, where
//!   the whole balance is disputed). wl-invoicing refuses what is not
//!   disputable and keeps the flag, the ticket and the paused reminders; a
//!   quality dispute becomes NCRs ([`crate::disputes`]).
//! - **What the buyer sees.** The status of the dispute in words (*Under
//!   review* while the invoice is flagged), never an NCR, a cost or another
//!   customer's data.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, InvoiceSummary, invoicer};
use wl_kernel::portal::portal_users;

use crate::disputes::QUALITY;
use crate::screens::{Texts, aed, date_text, done, options};
use crate::texts::{english, screen_texts, t, word};

/// The portal page of the invoices.
pub const INVOICES_URL: &str = "/portal/manufacturing/invoices";

/// The reasons the buyer chooses from.
pub const KINDS: &[&str] = &["quantity", "price", QUALITY, "other"];

/// A line the buyer may tick.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct LineBox {
    name: String,
    label: String,
}

/// An open invoice of the buyer's company.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct InvoiceRow {
    id: i64,
    number: String,
    balance: String,
    due: String,
    status: String,
    status_kind: &'static str,
    disputed: bool,
    lines: Vec<LineBox>,
}

/// What the invoices page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manufacturing/portal_invoices.html")]
pub(crate) struct InvoicesPage {
    tx: Texts,
    invoices: Vec<InvoiceRow>,
    kinds: Vec<(String, String)>,
    kind_selected: String,
}

/// The companies `user` acts for, or 403 when none.
async fn parties(state: &AppState, user: &AuthUser) -> Result<Vec<PartyId>> {
    let mut found: Vec<PartyId> = Vec::new();
    for grant in portal_users(state)
        .parties_of(&state.db, user.id, today())
        .await?
    {
        if !found.contains(&grant.acting_for) {
            found.push(grant.acting_for);
        }
    }
    if found.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(found)
}

/// The open invoices of the companies `user` acts for.
async fn open_invoices(state: &AppState, user: &AuthUser) -> Result<Vec<InvoiceSummary>> {
    let mut all = Vec::new();
    for party in parties(state, user).await? {
        all.extend(invoicer(state).open_of(&state.db, party).await?);
    }
    Ok(all)
}

/// `GET /portal/manufacturing/invoices`: the page for a browser, the rows as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut invoices = Vec::new();
    for summary in open_invoices(&state, &user).await? {
        let lines = invoicer(&state)
            .lines_of(&state.db, summary.id)
            .await?
            .into_iter()
            .map(|line| LineBox {
                name: format!("line_{}", line.position),
                label: format!(
                    "{}. {} ({})",
                    line.position,
                    line.description,
                    aed(line.gross)
                ),
            })
            .collect();
        invoices.push(InvoiceRow {
            id: summary.id.0,
            number: summary.number.clone().unwrap_or_default(),
            balance: aed(summary.balance),
            due: summary.due_date.map_or_else(String::new, date_text),
            status: if summary.disputed {
                t(&lang, "manufacturing.screen.portal.under_review")
            } else {
                word(&lang, "invoice_status", summary.status.as_str())
            },
            status_kind: if summary.disputed { "warning" } else { "info" },
            disputed: summary.disputed,
            lines,
        });
    }
    if !wl_kernel::negotiate::wants_html(&headers) {
        let rows: Vec<_> = invoices
            .iter()
            .map(|row| {
                renox::serde_json::json!({
                    "id": row.id, "number": row.number, "balance": row.balance,
                    "due": row.due, "status": row.status,
                })
            })
            .collect();
        return Ok(Json(rows).into_response());
    }
    let page = InvoicesPage {
        tx: screen_texts(&lang),
        invoices,
        kinds: options(&lang, "dispute_kind", KINDS),
        kind_selected: QUALITY.to_owned(),
    };
    Ok(View::typed(page).into_response())
}

/// `POST /portal/manufacturing/invoices/{id}/dispute`: `kind`, `text` and
/// `line_<position>` for each ticked line.
pub(crate) async fn dispute(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let invoice = open_invoices(&state, &user)
        .await?
        .into_iter()
        .find(|summary| summary.id.0 == id)
        .ok_or(Error::NotFound)?;
    let kind = form.get("kind").map_or("", |k| k.trim()).to_owned();
    if !KINDS.contains(&kind.as_str()) {
        return Err(crate::db::refused(english(
            "manufacturing.portal.kind",
            &[],
        )));
    }
    let mut request = DisputeRequest::new(
        InvoiceId(id),
        form.get("text").map_or("", |t| t.trim()),
        DisputeBy::Party { id: invoice.party },
    )
    .kind(kind.clone());
    if kind == QUALITY {
        let mut ticked: Vec<i32> = form
            .iter()
            .filter(|(_, value)| value.as_str() == "on")
            .filter_map(|(name, _)| name.strip_prefix("line_")?.parse().ok())
            .collect();
        if ticked.is_empty() {
            ticked = invoicer(&state)
                .lines_of(&state.db, invoice.id)
                .await?
                .iter()
                .map(|line| line.position)
                .collect();
        }
        request = request.lines(ticked);
    } else {
        request = request.amount(invoice.balance);
    }
    invoicer(&state).dispute(&state, request).await?;
    Ok(done(
        &lang,
        INVOICES_URL,
        "manufacturing.screen.done.disputed",
    ))
}
