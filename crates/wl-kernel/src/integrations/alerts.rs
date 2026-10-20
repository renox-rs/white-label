//! The alert when a connection goes `down` (wl-kernel §4.5 `go_down`,
//! `PLT.RUL.05`): the company admin and the holders of the connection's
//! owner role are told once, through the notice contract.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role;
use renox::prelude::*;
use renox::serde_json::json;

use super::SUBJECT_TYPE;
use super::connection::IntegrationConnection;
use crate::documents::dubai_text;
use crate::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest, Urgency};
use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::subject::Subject;
use crate::texts::en;

/// The notification type of the alert.
pub const NOTIFICATION_TYPE: &str = "kernel.integration_down";

/// The notification type `kernel.integration_down`.
pub fn notification_type() -> NotificationTypeDef {
    let subject = en("wl_kernel.integrations.alert.subject");
    let body = en("wl_kernel.integrations.alert.body");
    NotificationTypeDef::new(NOTIFICATION_TYPE, "kernel")
        .group("kernel")
        .category(Category::Reminder)
        .urgency(Urgency::Urgent)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// The users to tell about `connection`: the company admins and the holders
/// of its owner role, each once, by id.
pub async fn recipients(db: &Db, connection: &IntegrationConnection) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    for role in [COMPANY_ADMIN_ROLE, connection.owner_role.as_str()] {
        for user in users_with_role(db, role).await? {
            ids.insert(user.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// Tells the admin and the owner role that `connection` is `down`, with the
/// time the failures began (Asia/Dubai) and the manual fallback. Call it
/// once per `go_down`, after the commit.
pub async fn connection_down(state: &AppState, connection: &IntegrationConnection) -> Result {
    let users = recipients(&state.db, connection).await?;
    if users.is_empty() {
        return Ok(());
    }
    let since = connection
        .failed_since
        .map(dubai_text)
        .unwrap_or_else(|| "—".to_owned());
    let mut request =
        NotifyRequest::new(NOTIFICATION_TYPE, Subject::new(SUBJECT_TYPE, connection.id))
            .subject_label(&connection.key)
            .data(json!({
                "connection": connection.key,
                "since": since,
                "fallback": connection.fallback_en,
                "error": connection.last_error.clone().unwrap_or_default(),
            }));
    for id in users {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
