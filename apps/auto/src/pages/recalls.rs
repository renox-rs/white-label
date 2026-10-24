//! The recall page of the vertical (`BL-AUT-16`): the vehicle owners, fleets
//! and customers who got a recalled part. wl-inventory's recall page links to
//! it (its `recall_notices_route`, set by the host for this app).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/recalls/{id}` | `auto.recall` | `recalls.manage` |
//! | `POST /auto/recalls/{id}/send` | `auto.recall_send` | `recalls.manage` |
//!
//! Every line is *possible*: parts are not tracked by lot, so the unit sold or
//! fitted cannot be proved to come from the recalled receipts. The walk-in
//! sales of a branch and day are one line with their count.

use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::subject_label;
use wl_kernel::inventory::{RECALL_SUBJECT_TYPE, TraceMark};

use super::{Texts, party_name, require, text};
use crate::permissions;
use crate::recalls::{self, TraceRow};
use crate::texts;

/// The routes of the page.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/recalls/{id}", show)
        .name("auto.recall")
        .post("/auto/recalls/{id}/send", send)
        .name("auto.recall_send")
        .require_auth()
}

/// One line of the list, as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LineRow {
    /// The line of the recall.
    pub id: i64,
    /// The job card or sale, or the count of walk-in sales.
    pub record: String,
    /// The customer's name, or *walk-in sales at Branch 1*.
    pub party: String,
    /// The vehicle's plate for a job card.
    pub vehicle: String,
    /// The day.
    pub day: String,
    /// How sure the trace is, in words.
    pub mark: String,
    /// The badge kind of the mark.
    pub mark_kind: &'static str,
    /// Whether the party was told, in words.
    pub notice: String,
    /// The badge kind of the notice.
    pub notice_kind: &'static str,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/recall.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    back: String,
    rows: Vec<LineRow>,
    can_send: bool,
}

/// A row of the list in words.
async fn line_row(state: &AppState, lang: &Lang, row: &TraceRow) -> LineRow {
    let party = if row.walk_in {
        let count = if row.sales == 1 {
            text(lang, "recall.walk_in_one")
        } else {
            texts::t_with(lang, "auto.recall.walk_in_count", &[("count", &row.sales)])
        };
        format!(
            "{} · {} · {}",
            text(lang, "recall.walk_in"),
            row.branch,
            count
        )
    } else {
        match row.party {
            Some(party) => party_name(state, &lang.locale, party).await,
            None => text(lang, "recall.party_unknown"),
        }
    };
    let (mark, mark_kind) = match row.mark {
        TraceMark::Confirmed => (text(lang, "recall.mark_confirmed"), "info"),
        TraceMark::Possible | TraceMark::Uncertain => {
            (text(lang, "recall.mark_possible"), "warning")
        }
    };
    let (notice, notice_kind) = match row.notice.as_str() {
        "sent" => (text(lang, "recall.notice_sent"), "success"),
        "waived" => (text(lang, "recall.notice_waived"), "info"),
        _ => (text(lang, "recall.notice_pending"), "warning"),
    };
    LineRow {
        id: row.line.subject_id,
        record: row.record.clone(),
        party,
        vehicle: row.vehicle.clone().unwrap_or_default(),
        day: row.on.map(|day| day.to_string()).unwrap_or_default(),
        mark,
        mark_kind,
        notice,
        notice_kind,
    }
}

/// `GET /auto/recalls/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::RECALLS_MANAGE)?;
    let found = recalls::trace(&state, id).await?;
    let mut rows = Vec::new();
    for row in &found {
        rows.push(line_row(&state, &lang, row).await);
    }
    let number = subject_label(&state.db, &Subject::new(RECALL_SUBJECT_TYPE, id))
        .await?
        .unwrap_or_else(|| format!("Recall #{id}"));
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id,
        number,
        back: format!("/recalls/{id}"),
        can_send: found
            .iter()
            .any(|row| row.notice == "pending" && !row.walk_in && row.addressee.is_some()),
        rows,
    }))
}

/// `POST /auto/recalls/{id}/send`.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    recalls::send(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "recall.sent")), renox::HxRefresh).into_response())
}
