//! The back office's part of compliance alerts (wl-kernel §5.15): the page of
//! the open alerts the viewer may act on, and the route that records how the
//! duty was met by hand.
//!
//! - `GET /compliance-alerts`: the open alerts of the rules whose owner role
//!   the viewer holds, and, for a company admin, all of them.
//! - `POST /compliance-alerts/{id}/close` (`note` required, `attachment`
//!   optional: the id of a document): [`close_by_hand`](super::close_by_hand).

use std::collections::BTreeMap;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::alert::{AlertKind, ComplianceAlert};
use super::service::close_by_hand;
use crate::documents::dubai_text;
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::texts;

/// The name of the list route.
pub const LIST_ROUTE: &str = "wl_kernel.compliance";

/// The name of the close route.
pub const CLOSE_ROUTE: &str = "wl_kernel.compliance.close";

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// One open alert as the page lists it.
#[derive(Debug, Clone, Serialize)]
pub struct AlertRow {
    /// The alert's id.
    pub id: i64,
    /// The rule id.
    pub rule: String,
    /// What happened, in words.
    pub what: String,
    /// The record the run is about.
    pub subject: String,
    /// When the alert was raised (Asia/Dubai).
    pub raised_at: String,
    /// The owner's task number, or `—`.
    pub task: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/compliance_alerts.html")]
pub struct ComplianceAlertsPage {
    tx: Texts,
    alerts: Vec<AlertRow>,
}

/// The routes, for logged-in users; who may close an alert is checked per
/// request (403 otherwise).
pub fn routes() -> Routes {
    Routes::new()
        .get("/compliance-alerts", list)
        .name(LIST_ROUTE)
        .post("/compliance-alerts/{id}/close", close)
        .name(CLOSE_ROUTE)
        .require_auth()
}

/// The page's texts.
fn page_texts(lang: &Lang) -> Texts {
    [
        "title",
        "subtitle",
        "rule",
        "what",
        "subject",
        "raised_at",
        "task",
        "actions",
        "close",
        "close_title",
        "note",
        "note_hint",
        "attachment",
        "attachment_hint",
        "empty",
    ]
    .into_iter()
    .map(|key| {
        (
            key,
            texts::t(lang, &format!("wl_kernel.compliance.page.{key}")),
        )
    })
    .collect()
}

/// What happened, as a row shows it.
fn what_of(lang: &Lang, alert: &ComplianceAlert) -> String {
    let key = match alert.kind {
        AlertKind::Failed => "failed",
        AlertKind::Stopped => "stopped",
        AlertKind::Missed => "missed",
    };
    texts::t(lang, &format!("wl_kernel.compliance.kind.{key}"))
}

async fn list(State(db): State<Db>, user: AuthUser, lang: Lang) -> Result<View> {
    let admin = user.has_role(COMPANY_ADMIN_ROLE);
    let alerts = ComplianceAlert::open(&db)
        .await?
        .into_iter()
        .filter(|alert| admin || user.has_role(&alert.owner_role))
        .map(|alert| AlertRow {
            id: alert.id,
            rule: alert.rule.clone(),
            what: what_of(&lang, &alert),
            subject: alert.subject().to_string(),
            raised_at: alert.created_at.map_or_else(|| "—".to_owned(), dubai_text),
            task: alert
                .ticket_number
                .clone()
                .unwrap_or_else(|| "—".to_owned()),
        })
        .collect();
    Ok(View::typed(ComplianceAlertsPage {
        tx: page_texts(&lang),
        alerts,
    }))
}

/// The form of the close action.
#[derive(Debug, Default, Deserialize, Serialize)]
struct CloseForm {
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    attachment: Option<String>,
}

impl Validate for CloseForm {
    fn rules(&self, _v: &mut Validator) {}
}

async fn close(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path(id): Path<i64>,
    Valid(form): Valid<CloseForm>,
) -> Result<Response> {
    let note = form.note.as_deref().map_or("", str::trim);
    if note.is_empty() {
        let mut errors = Errors::new();
        errors.add("note", texts::t(&lang, "wl_kernel.compliance.note_required"));
        return Err(ValidationError::new(errors).with_input(&form).into());
    }
    let attachment = match form.attachment.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => match text.parse::<i64>() {
            Ok(document) => Some(document),
            Err(_) => {
                let mut errors = Errors::new();
                errors.add(
                    "attachment",
                    texts::t(&lang, "wl_kernel.compliance.attachment_invalid"),
                );
                return Err(ValidationError::new(errors).with_input(&form).into());
            }
        },
    };
    close_by_hand(&state, &user, id, note, attachment).await?;
    let toast = Toast::success(texts::t(&lang, "wl_kernel.compliance.toast.closed"));
    Ok((toast, back).into_response())
}
