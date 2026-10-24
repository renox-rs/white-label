//! The back office's part of compliance alerts (wl-kernel §5.15): the page of
//! the open alerts the viewer may act on, and the route that records how the
//! duty was met by hand.
//!
//! - `GET /compliance-alerts`: the open alerts of the rules whose owner role
//!   the viewer holds, and, for a company admin, all of them.
//! - `POST /compliance-alerts/{id}/close` (`note` required, `attachment`
//!   optional: the id of a document): [`close_by_hand`](super::close_by_hand).

use std::collections::BTreeMap;

use renox::grid::{Action, Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::alert::{AlertKind, ComplianceAlert};
use super::service::close_by_hand;
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::texts;

/// The name of the list route.
pub const LIST_ROUTE: &str = "wl_kernel.compliance";

/// The name of the close route.
pub const CLOSE_ROUTE: &str = "wl_kernel.compliance.close";

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/compliance_alerts.html")]
pub struct ComplianceAlertsPage {
    tx: Texts,
    alerts: GridPage<ComplianceAlert>,
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

/// The grid of the open alerts: search on the rule, filters by rule and
/// kind, sorted by when they were raised. The record and the close button
/// (a row action that opens the sheet of the page) are the grid's own.
fn alerts_grid(lang: &Lang) -> Grid {
    let t = |key: &str| texts::t(lang, &format!("wl_kernel.compliance.page.{key}"));
    let kinds: Vec<(String, String)> = [
        ("failed", AlertKind::Failed),
        ("stopped", AlertKind::Stopped),
        ("missed", AlertKind::Missed),
    ]
    .iter()
    .map(|(key, kind)| {
        (
            kind.as_str().to_owned(),
            texts::t(lang, &format!("wl_kernel.compliance.kind.{key}")),
        )
    })
    .collect();
    Grid::new("kernel-compliance-alerts")
        .title(&t("title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("rule", &t("rule"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(
            Column::select("kind", &t("what"), kinds)
                .badges(&[
                    ("failed", "danger"),
                    ("stopped", "danger"),
                    ("missed", "warning"),
                ])
                .mobile(),
        )
        .column(Column::custom("subject", &t("subject")).mobile())
        .column(Column::datetime("created_at", &t("raised_at")))
        .column(Column::text("ticket_number", &t("task")).searchable())
        .column(Column::text("owner_role", &t("owner_role")).hidden())
        .column(Column::text("error", &t("error")).hidden())
        .sort_by("created_at")
        .cards_on_mobile()
        .row_action(Action::new(&t("close"), "/compliance-alerts/{id}/close").sheet("close-alert"))
        .empty_state(&t("empty"), None)
}

async fn list(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    // The alerts the viewer may act on: a company admin all, anyone else the
    // ones of the rules whose owner role they hold.
    let admin = user.has_role(COMPANY_ADMIN_ROLE);
    let visible: Vec<i64> = ComplianceAlert::open(&db)
        .await?
        .into_iter()
        .filter(|alert| admin || user.has_role(&alert.owner_role))
        .map(|alert| alert.id)
        .collect();
    let query = ComplianceAlert::query().where_(ComplianceAlert::ID.is_in(if visible.is_empty() {
        vec![-1]
    } else {
        visible
    }));
    let alerts = alerts_grid(&lang)
        .page(query, &request)
        .await?
        .extend(|alert| json!({ "subject": alert.subject().to_string() }));
    Ok(View::typed(ComplianceAlertsPage {
        tx: page_texts(&lang),
        alerts,
    })
    .into_response())
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
        errors.add(
            "note",
            texts::t(&lang, "wl_kernel.compliance.note_required"),
        );
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
