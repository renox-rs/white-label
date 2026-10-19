//! The vertical's notification types and who is told (01 Property §7.2),
//! sent through the kernel [`Notifier`](wl_kernel::notify::Notifier). Types
//! that go only to staff are registered with the reaction `none`: the item
//! stays on the user's task list when a message gives up.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role_in;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{
    self, Category, GROUP_PORTAL, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};

use crate::lease::Lease;
use crate::permissions::{LEASING_AGENT_ROLE, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The leasing agent and the property manager are reminded that a lease is
/// still waiting for its signature (`PRP.RUL.12`, at N days).
pub const LEASE_UNSIGNED: &str = "property.lease_unsigned";

/// The property manager is told that the lease entered its renewal window
/// (`PRP.RUL.01`).
pub const RENEWAL_DUE: &str = "property.renewal_due";

/// The property manager is warned that the notice-of-changed-terms deadline
/// is near and the renewal decision or the notice is missing (`PRP.RUL.09`,
/// 14 and 3 days before).
pub const RENEWAL_DEADLINE: &str = "property.renewal_deadline";

/// The property manager is told that the deadline passed and the lease can
/// now be renewed on the current terms only (`PRP.RUL.09`).
pub const CURRENT_TERMS_ONLY: &str = "property.current_terms_only";

/// The property managers are told that the tenant holds over after the end
/// date (`PRP.RUL.03`).
pub const HOLDING_OVER: &str = "property.holding_over";

/// The tenant is told that the lease entered its renewal window: a normal
/// message, not a statutory notice (`PRP.RUL.01`).
pub const RENEWAL_WINDOW: &str = "lease.renewal_window";

/// The statutory notice of changed terms to the tenant (`PRP.RUL.09`,
/// `PRP.ACL.17`). The property manager sends it; its delivery is the
/// provider's receipt or the proof staff record when it gave up.
pub const NOTICE_OF_CHANGED_TERMS: &str = "lease.notice_of_changed_terms";

/// The statutory notice of a landlord who ends the lease (`record_notice`
/// with party landlord). Its delivery is the provider's receipt or the proof
/// staff record when it gave up; the planned move-out date and the deadlines
/// that depend on the notice start from it.
pub const LANDLORD_NOTICE: &str = "lease.landlord_notice";

/// The property managers are told that a terminated lease still has its
/// Ejari or Tawtheeq registration to cancel (`PRP.RUL.13`).
pub const EJARI_CANCELLATION: &str = "property.ejari_cancellation";

/// An e-mail reminder type of the vertical with English texts.
fn english_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    let subject = english(&format!("property.notices.{texts}_subject"), &[]);
    let body = english(&format!("property.notices.{texts}_body"), &[]);
    NotificationTypeDef::new(key, "property")
        .group("property")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// A type that goes only to staff: the item stays on the user's task list when
/// a message gives up, so the reaction is `none` (§7.2 *Notification types*).
fn staff_reminder(key: &str, texts: &str) -> NotificationTypeDef {
    english_reminder(key, texts).give_up(&[GiveUpReaction::None])
}

/// A message to the tenant with English texts; `statutory` for a notice with
/// legal effect, whose give-up the platform handles with the proof task.
fn tenant_message(key: &str, texts: &str, statutory: bool) -> NotificationTypeDef {
    let subject = english(&format!("property.notices.{texts}_subject"), &[]);
    let body = english(&format!("property.notices.{texts}_body"), &[]);
    let def = NotificationTypeDef::new(key, "property")
        .template(NotifyChannel::Email, "en", Some(&subject), &body);
    if statutory {
        def.group("statutory").statutory(true)
    } else {
        def.group(GROUP_PORTAL)
    }
}

/// Registers the vertical's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(english_reminder(LEASE_UNSIGNED, "lease_unsigned"));
    notify::register_type(staff_reminder(RENEWAL_DUE, "renewal_due"));
    notify::register_type(staff_reminder(RENEWAL_DEADLINE, "renewal_deadline"));
    notify::register_type(staff_reminder(CURRENT_TERMS_ONLY, "current_terms_only"));
    notify::register_type(staff_reminder(HOLDING_OVER, "holding_over"));
    notify::register_type(tenant_message(RENEWAL_WINDOW, "renewal_window", false));
    notify::register_type(tenant_message(
        NOTICE_OF_CHANGED_TERMS,
        "changed_terms",
        true,
    ));
    notify::register_type(tenant_message(LANDLORD_NOTICE, "landlord_notice", true));
    notify::register_type(staff_reminder(EJARI_CANCELLATION, "ejari_cancellation"));
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

/// The users who hold the property manager role in the building of `lease`,
/// or everywhere.
pub async fn managers(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    holders(state, PROPERTY_MANAGER_ROLE, lease).await
}

/// Tells the property managers of `lease` with a message of `type_key`; no
/// message when nobody holds the role. Returns who was told.
async fn tell_managers(
    state: &AppState,
    lease: &Lease,
    type_key: &str,
    data: renox::serde_json::Value,
) -> Result<Vec<i64>> {
    let users = managers(state, lease).await?;
    if users.is_empty() {
        return Ok(users);
    }
    let mut request = NotifyRequest::new(type_key, lease.subject())
        .subject_label(&lease.number_with_version())
        .data(data);
    for user in &users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(users)
}

/// A date as the messages show it: `2 Oct 2027`.
fn shown(day: NaiveDate) -> String {
    day.format("%-d %b %Y").to_string()
}

/// `lease` entered its renewal window (`PRP.RUL.01`): the property managers
/// are told with the notice deadline, and the tenant gets a normal message.
pub async fn renewal_due(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "end_date": shown(lease.end_date),
        "deadline": shown(deadline),
    });
    tell_managers(state, lease, RENEWAL_DUE, data.clone()).await?;
    let request = NotifyRequest::new(RENEWAL_WINDOW, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(data);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Warns the property managers that the notice deadline of `lease` is `days`
/// days away and `problem` is not settled (`PRP.RUL.09`).
pub async fn renewal_deadline(
    state: &AppState,
    lease: &Lease,
    days: i64,
    deadline: NaiveDate,
    problem: &str,
) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "days": days,
        "deadline": shown(deadline),
        "problem": problem,
    });
    tell_managers(state, lease, RENEWAL_DEADLINE, data).await?;
    Ok(())
}

/// Tells the property managers that the deadline of `lease` passed without a
/// timely notice, so only the current terms can be renewed (`PRP.RUL.09`).
pub async fn current_terms_only(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "deadline": shown(deadline),
    });
    tell_managers(state, lease, CURRENT_TERMS_ONLY, data).await?;
    Ok(())
}

/// Sends the statutory notice of changed terms of `lease` to the tenant.
pub async fn changed_terms(state: &AppState, lease: &Lease, deadline: NaiveDate) -> Result {
    let request = NotifyRequest::new(NOTICE_OF_CHANGED_TERMS, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number_with_version(),
            "end_date": shown(lease.end_date),
            "deadline": shown(deadline),
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Sends the statutory notice of the landlord to end `lease` to the tenant:
/// `notice_date` is the date of the notice and `ground` the landlord's ground
/// as text.
pub async fn landlord_notice(
    state: &AppState,
    lease: &Lease,
    notice_date: NaiveDate,
    ground: &str,
) -> Result {
    let request = NotifyRequest::new(LANDLORD_NOTICE, lease.subject())
        .subject_label(&lease.number_with_version())
        .to_party(lease.tenant())
        .data(json!({
            "lease": lease.number_with_version(),
            "end_date": shown(lease.end_date),
            "notice_date": shown(notice_date),
            "ground": ground,
        }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the property managers that the Ejari or Tawtheeq registration of the
/// terminated `lease` is not cancelled yet (`PRP.RUL.13`). Returns who was
/// told.
pub async fn ejari_cancellation(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    let data = json!({
        "lease": lease.number_with_version(),
        "ejari_number": lease.ejari_number.clone().unwrap_or_default(),
        "terminated_on": lease.terminated_on.map(shown).unwrap_or_default(),
    });
    tell_managers(state, lease, EJARI_CANCELLATION, data).await
}

/// Tells the property managers that `lease` is holding over until the end of
/// its statutory renewal period (`PRP.RUL.03`).
pub async fn holding_over(state: &AppState, lease: &Lease) -> Result {
    let data = json!({
        "lease": lease.number_with_version(),
        "end_date": shown(lease.end_date),
        "until": lease.holding_over_until.map(shown).unwrap_or_default(),
    });
    tell_managers(state, lease, HOLDING_OVER, data).await?;
    Ok(())
}
