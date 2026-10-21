//! The pages of a recall after it is opened (wl-inventory §4 *Recall*, issues
//! #234 and #235), all for a holder of `recalls.manage`:
//!
//! | Route | Name | What |
//! |---|---|---|
//! | `POST /recalls/{id}/trace` | `wl_inventory.recall_trace` | runs the trace again |
//! | `GET /recalls/{id}/lines` | `wl_inventory.recall_lines` | the affected lines (JSON): subject, party, lot, quantity, mark and notice status |
//! | `POST /recalls/{id}/notices` | `wl_inventory.recall_notices` | sends the `template` notice to the `lines` |
//! | `POST /recall-lines/{id}/waive` | `wl_inventory.recall_line_waive` | waives a line with a `reason` |
//! | `POST /recalls/{id}/close` | `wl_inventory.recall_close` | closes the recall and keeps its report |
//! | `GET /recalls/{id}/report` | `wl_inventory.recall_report` | the report (PDF) of a closed recall |
//!
//! The routes need a signed-in user; the handlers and the transitions check
//! the permission.

use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::permissions;
use crate::recall::Recall;
use crate::recall_line::{RecallLine, lines_of};
use crate::recall_notice::{send_notices, waive};
use crate::recall_trace::trace;
use crate::texts;

/// The routes of this file.
pub fn routes() -> Routes {
    Routes::new()
        .post("/recalls/{id}/trace", trace_again)
        .name("wl_inventory.recall_trace")
        .get("/recalls/{id}/lines", affected_lines)
        .name("wl_inventory.recall_lines")
        .post("/recalls/{id}/notices", send)
        .name("wl_inventory.recall_notices")
        .post("/recall-lines/{id}/waive", waive_line)
        .name("wl_inventory.recall_line_waive")
        .post("/recalls/{id}/close", close_recall)
        .name("wl_inventory.recall_close")
        .get("/recalls/{id}/report", report)
        .name("wl_inventory.recall_report")
        .require_auth()
}

/// The package text `key`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_inventory.{key}"))
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A transition's refusal as an answer.
fn answer(error: TransitionError) -> Error {
    match error {
        TransitionError::Guard(message) => refused(message),
        other => other.into_error(),
    }
}

/// `Forbidden` unless `user` holds `recalls.manage`.
fn ensure_manager(user: &AuthUser) -> Result {
    if user.has_permission(permissions::RECALLS_MANAGE) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// `POST /recalls/{id}/trace`: follows the movements again; existing lines
/// keep their notice status.
async fn trace_again(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    trace(&state, Some(&user), id).await.map_err(answer)?;
    Ok((
        Toast::success(text(&lang, "recall.traced")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What the lines screen reads.
#[derive(Serialize)]
struct LinesView {
    /// The recall's number.
    number: String,
    /// Where the recall stands.
    status: String,
    /// The affected lines.
    lines: Vec<wl_kernel::inventory::TracedHolder>,
}

/// `GET /recalls/{id}/lines`: the affected lines as JSON.
async fn affected_lines(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    ensure_manager(&user)?;
    let recall = Recall::find_or_404(&state.db, id).await?;
    let view = LinesView {
        number: recall.number.clone().unwrap_or_default(),
        status: recall.status.as_str().to_owned(),
        lines: lines_of(&state.db, id)
            .await?
            .iter()
            .map(RecallLine::holder)
            .collect(),
    };
    let body = renox::serde_json::to_string(&view)
        .map_err(|error| Error::Internal(renox::anyhow::Error::new(error)))?;
    Ok(([("content-type", "application/json")], body).into_response())
}

/// What the owner enters to send notices.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct NoticeForm {
    /// `customer`, `patient` or `internal`.
    #[validate(max = 32)]
    pub template: Option<String>,
    /// The ids of the lines to tell, like `3, 4`.
    #[validate(max = 500)]
    pub lines: Option<String>,
}

/// The ids in `text` (`3, 4`); `None` when one is not a number.
fn ids(text: &Option<String>) -> Option<Vec<i64>> {
    let mut found = Vec::new();
    for part in text.as_deref().unwrap_or_default().split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        found.push(part.parse::<i64>().ok()?);
    }
    Some(found)
}

/// `POST /recalls/{id}/notices`: tells the parties of the lines selected.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<NoticeForm>,
) -> Result<Response> {
    let lines = ids(&form.lines).ok_or_else(|| refused(text(&lang, "recall.bad_form")))?;
    send_notices(
        &state,
        &user,
        id,
        form.template.as_deref().unwrap_or_default(),
        lines,
    )
    .await
    .map_err(answer)?;
    Ok((
        Toast::success(text(&lang, "recall.notices_sent")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What the owner enters to waive a line.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct WaiveForm {
    /// Why the party need not be told.
    #[validate(max = 500)]
    pub reason: Option<String>,
}

/// `POST /recall-lines/{id}/waive`: the party need not be told.
async fn waive_line(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<WaiveForm>,
) -> Result<Response> {
    waive(
        &state,
        &user,
        id,
        form.reason.as_deref().unwrap_or_default(),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "recall.waived")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /recalls/{id}/close`: closes the recall and keeps the report.
async fn close_recall(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    crate::recall_close::close(&state, &user, id)
        .await
        .map_err(answer)?;
    Ok((
        Toast::success(text(&lang, "recall.closed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `GET /recalls/{id}/report`: the report of a closed recall, a PDF.
async fn report(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    ensure_manager(&user)?;
    let recall = Recall::find_or_404(&state.db, id).await?;
    let report = recall.report.ok_or(Error::NotFound)?;
    Ok(([("content-type", "application/pdf")], report).into_response())
}
