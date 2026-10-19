//! The back office's part of integrations (wl-kernel §4.5, §5.14): the
//! banner on the screens a `down` connection affects, and the pages of the
//! connections.
//!
//! - **Banner.** A screen registered with a connection ([`IntegrationDef::screens`](super::IntegrationDef::screens))
//!   gives its template `integration_banner` ([`banner_for`]) and includes
//!   the partial:
//!   ```html
//!   <rx-include template="wl-kernel/integration_banner.html"/>
//!   ```
//!   It shows the manual fallback in the viewer's language (English when the
//!   Arabic text is empty), only for screens the connection registered, and
//!   only while the connection is `down`.
//! - **Pages.** `GET /integrations` and `GET /integrations/{key}` for holders
//!   of `integrations.view`; `POST /integrations/{key}/disable` (a reason is
//!   required) and `POST /integrations/{key}/enable` for holders of
//!   `integrations.manage`.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use super::connection::{ConnectionStatus, IntegrationConnection};
use super::queue::QueuedCall;
use super::service;
use crate::documents::dubai_text;
use crate::permissions::{INTEGRATIONS_MANAGE, INTEGRATIONS_VIEW};
use crate::texts;

/// The template of the banner, compiled into the kernel.
pub const BANNER_TEMPLATE: &str = "wl-kernel/integration_banner.html";

/// The names of the routes.
pub mod route_names {
    /// `GET /integrations`.
    pub const LIST: &str = "wl_kernel.integrations";
    /// `GET /integrations/{key}`.
    pub const SHOW: &str = "wl_kernel.integrations.show";
    /// `POST /integrations/{key}/disable`.
    pub const DISABLE: &str = "wl_kernel.integrations.disable";
    /// `POST /integrations/{key}/enable`.
    pub const ENABLE: &str = "wl_kernel.integrations.enable";
}

/// The texts a template reads as `tx['key']`.
type Texts = BTreeMap<&'static str, String>;

/// The banners of `screen` for a viewer reading `lang`: one per `down`
/// connection that registered the screen, with its manual fallback. `None`
/// when there is none, so the partial shows nothing.
pub async fn banner_for(db: &Db, screen: &str, lang: &Lang) -> Result<Option<Value>> {
    let down = IntegrationConnection::query()
        .where_(IntegrationConnection::STATUS.eq(ConnectionStatus::Down))
        .order_by(IntegrationConnection::ID)
        .get(db)
        .await?;
    let banners: Vec<Value> = down
        .iter()
        .filter(|connection| connection.affects(screen))
        .map(|connection| {
            renox::serde_json::json!({
                "connection": connection.key,
                "text": texts::t_with(
                    lang,
                    "wl_kernel.integrations.banner",
                    &[
                        ("connection", &connection.key),
                        ("fallback", &connection.fallback_for(&lang.locale)),
                    ],
                ),
            })
        })
        .collect();
    Ok((!banners.is_empty()).then(|| Value::Array(banners)))
}

/// The label and colour of a status.
fn status_of(lang: &Lang, status: ConnectionStatus) -> (String, &'static str) {
    let kind = match status {
        ConnectionStatus::Up => "success",
        ConnectionStatus::Degraded => "warning",
        ConnectionStatus::Down => "error",
        ConnectionStatus::Disabled => "neutral",
    };
    (
        texts::t(
            lang,
            &format!("wl_kernel.integrations.status.{}", status.as_text()),
        ),
        kind,
    )
}

/// `at` in Asia/Dubai, or `—`.
fn when(at: Option<DateTime>) -> String {
    at.map_or_else(|| "—".to_owned(), dubai_text)
}

/// The text `key` of the pages.
fn page_texts(lang: &Lang, keys: &[&'static str]) -> Texts {
    keys.iter()
        .map(|key| {
            (
                *key,
                texts::t(lang, &format!("wl_kernel.integrations.page.{key}")),
            )
        })
        .collect()
}

/// One connection in the list.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionRow {
    /// The key.
    pub key: String,
    /// The owning module.
    pub module: String,
    /// The page of the connection.
    pub url: String,
    /// The status, in words.
    pub status_label: String,
    /// The badge's colour.
    pub status_kind: &'static str,
    /// When the failures began, or `—`.
    pub failed_since: String,
    /// When a call last succeeded, or `—`.
    pub last_success: String,
    /// How many calls are queued.
    pub queued: i64,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/integrations.html")]
pub struct IntegrationsPage {
    tx: Texts,
    connections: Vec<ConnectionRow>,
}

/// One queued call on a connection's page.
#[derive(Debug, Clone, Serialize)]
pub struct CallRow {
    /// The call's id.
    pub id: i64,
    /// What the call is.
    pub kind: String,
    /// The record it is about, or `—`.
    pub subject: String,
    /// When it was queued.
    pub enqueued_at: String,
    /// Failed attempts.
    pub attempts: i64,
    /// When it is tried next, or `—`.
    pub next_attempt: String,
    /// The error of the last failed attempt, or `—`.
    pub last_error: String,
}

/// What a connection's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-kernel/integration.html")]
pub struct IntegrationPage {
    tx: Texts,
    heading: String,
    key: String,
    module: String,
    owner_role: String,
    down_after: String,
    status_label: String,
    status_kind: String,
    is_down: bool,
    fallback: String,
    screens: String,
    last_success: String,
    failed_since: String,
    last_error: String,
    disabled_reason: String,
    can_disable: bool,
    can_enable: bool,
    queue: Vec<CallRow>,
}

/// The routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/integrations", list)
        .name(route_names::LIST)
        .get("/integrations/{key}", show)
        .name(route_names::SHOW)
        .require_permission(INTEGRATIONS_VIEW)
        .merge(
            Routes::new()
                .post("/integrations/{key}/disable", disable)
                .name(route_names::DISABLE)
                .post("/integrations/{key}/enable", enable)
                .name(route_names::ENABLE)
                .require_permission(INTEGRATIONS_MANAGE),
        )
}

async fn list(State(db): State<Db>, lang: Lang) -> Result<View> {
    let connections = IntegrationConnection::query()
        .order_by(IntegrationConnection::KEY)
        .get(&db)
        .await?;
    let mut rows = Vec::with_capacity(connections.len());
    for connection in connections {
        let (status_label, status_kind) = status_of(&lang, connection.status);
        let queued = QueuedCall::waiting(&db, &connection.key).await?.len();
        rows.push(ConnectionRow {
            url: format!("/integrations/{}", connection.key),
            module: connection.module.clone(),
            status_label,
            status_kind,
            failed_since: when(connection.failed_since),
            last_success: when(connection.last_success_at),
            queued: i64::try_from(queued).unwrap_or(i64::MAX),
            key: connection.key,
        });
    }
    Ok(View::typed(IntegrationsPage {
        tx: page_texts(
            &lang,
            &[
                "title",
                "subtitle",
                "key",
                "module",
                "status",
                "failed_since",
                "last_success",
                "queued",
                "empty",
            ],
        ),
        connections: rows,
    }))
}

async fn show(
    State(db): State<Db>,
    user: AuthUser,
    lang: Lang,
    Path(key): Path<String>,
) -> Result<View> {
    let connection = IntegrationConnection::by_key(&db, &key)
        .await?
        .ok_or(Error::NotFound)?;
    let (status_label, status_kind) = status_of(&lang, connection.status);
    let calls = QueuedCall::waiting(&db, &connection.key).await?;
    let queue = calls
        .iter()
        .map(|call| CallRow {
            id: call.id,
            kind: call.kind.clone(),
            subject: call
                .subject()
                .map_or_else(|| "—".to_owned(), |subject| subject.to_string()),
            enqueued_at: dubai_text(call.enqueued_at),
            attempts: call.attempts,
            next_attempt: when(call.next_attempt_at),
            last_error: call.last_error.clone().unwrap_or_else(|| "—".to_owned()),
        })
        .collect();
    let can_manage = user.has_permission(INTEGRATIONS_MANAGE);
    Ok(View::typed(IntegrationPage {
        tx: page_texts(
            &lang,
            &[
                "module",
                "disable",
                "disable_title",
                "reason",
                "reason_hint",
                "enable",
                "enable_title",
                "enable_message",
                "cancel",
                "owner_role",
                "down_after",
                "last_success",
                "failed_since",
                "last_error",
                "disabled_reason",
                "fallback",
                "screens",
                "queue",
                "call",
                "subject",
                "enqueued_at",
                "attempts",
                "next_attempt",
                "queue_empty",
            ],
        ),
        heading: connection.key.clone(),
        module: connection.module.clone(),
        owner_role: connection.owner_role.clone(),
        down_after: texts::t_with(
            &lang,
            "wl_kernel.integrations.page.minutes",
            &[("minutes", &connection.down_after_minutes)],
        ),
        status_label,
        status_kind: status_kind.to_owned(),
        is_down: connection.status == ConnectionStatus::Down,
        fallback: connection.fallback_for(&lang.locale).to_owned(),
        screens: connection.screens.join(", "),
        last_success: when(connection.last_success_at),
        failed_since: when(connection.failed_since),
        last_error: connection
            .last_error
            .clone()
            .unwrap_or_else(|| "—".to_owned()),
        disabled_reason: connection
            .disabled_reason
            .clone()
            .unwrap_or_else(|| "—".to_owned()),
        can_disable: can_manage && connection.status != ConnectionStatus::Disabled,
        can_enable: can_manage && connection.status == ConnectionStatus::Disabled,
        queue,
        key: connection.key,
    }))
}

/// The form of the disable action.
#[derive(Debug, Default, Deserialize, Serialize)]
struct DisableForm {
    #[serde(default)]
    reason: Option<String>,
}

impl Validate for DisableForm {
    fn rules(&self, _v: &mut Validator) {}
}

async fn disable(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path(key): Path<String>,
    Valid(form): Valid<DisableForm>,
) -> Result<Response> {
    let reason = form.reason.as_deref().map_or("", str::trim);
    if reason.is_empty() {
        let mut errors = Errors::new();
        errors.add(
            "reason",
            texts::t(&lang, "wl_kernel.integrations.reason_required"),
        );
        return Err(ValidationError::new(errors).with_input(&form).into());
    }
    service::disable(&state, &user, &key, reason)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    let toast = Toast::success(texts::t(&lang, "wl_kernel.integrations.toast.disabled"));
    Ok((toast, back).into_response())
}

async fn enable(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back: Back,
    Path(key): Path<String>,
) -> Result<Response> {
    service::enable(&state, &user, &key)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    let toast = Toast::success(texts::t(&lang, "wl_kernel.integrations.toast.enabled"));
    Ok((toast, back).into_response())
}
