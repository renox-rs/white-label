//! The vertical's notification types and who is told (01 Property §7.2),
//! sent through the kernel [`Notifier`](wl_kernel::notify::Notifier). Types
//! that go only to staff are registered with the reaction `none`: the item
//! stays on the user's task list when a message gives up.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role_in;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::lease::Lease;
use crate::permissions::{LEASING_AGENT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The leasing agent and the property manager are reminded that a lease is
/// still waiting for its signature (`PRP.RUL.12`, at N days).
pub const LEASE_UNSIGNED: &str = "property.lease_unsigned";

/// An e-mail reminder type of the vertical with English texts.
fn english_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("property.notices.{texts}_subject"), &[]);
    let body = english(&format!("property.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "property")
        .group("property")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the vertical's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_reminder(LEASE_UNSIGNED, "lease_unsigned"));
}

/// The users who hold `role` in the building of `lease`, or everywhere.
async fn holders(state: &AppState, role: &str, lease: &Lease) -> Result<Vec<i64>> {
    Ok(users_with_role_in(&state.db, role, &lease.scope())
        .await?
        .iter()
        .map(|user| user.id)
        .collect())
}

/// The leasing agent of `lease` (who drafted it, else the holders of the
/// leasing agent role in its building) and its property managers.
pub async fn leasing_team(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    match lease.created_by {
        Some(agent) => {
            ids.insert(agent);
        }
        None => ids.extend(holders(state, LEASING_AGENT_ROLE, lease).await?),
    }
    ids.extend(holders(state, PROPERTY_MANAGER_ROLE, lease).await?);
    Ok(ids.into_iter().collect())
}

/// Reminds the leasing agent and the property managers that `lease` has been
/// waiting `days` days for its signature. Nothing is sent to the tenant.
pub async fn lease_unsigned(
    state: &AppState,
    lease: &Lease,
    days: i64,
    since: NaiveDate,
) -> Result<Vec<i64>> {
    let users = leasing_team(state, lease).await?;
    if users.is_empty() {
        return Ok(users);
    }
    let mut request = NotifyRequest::new(LEASE_UNSIGNED, lease.subject())
        .subject_label(&lease.number_with_version())
        .data(json!({
            "lease": lease.number_with_version(),
            "days": days,
            "since": since.format("%-d %b %Y").to_string(),
        }));
    for user in &users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users)
}
