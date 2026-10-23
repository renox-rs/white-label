//! Notices to the staff of the plant (07 Manufacturing §7.1): the planner is
//! told when a work order is unlinked, sales when one is aborted. The alerts
//! of the work order go to staff users only: the type names no group and
//! takes `none` on give-up (staff always also get the in-app notification).

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{Subject, role_tasks};

use crate::permissions;
use crate::texts::template;

/// The notification type of a notice to the staff.
pub const NOTICE: &str = "manufacturing.notice";

/// The staff notification type: a title and a body the caller fills in.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "manufacturing")
        .group("")
        .category(Category::Reminder)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manufacturing.notice.subject")),
            template("manufacturing.notice.body"),
        )
}

/// Registers the notification types.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The holders of `role` in `plant` or anywhere, else whom the kernel picks
/// for the role; empty when nobody holds it.
///
/// # Errors
///
/// A database error.
pub(crate) async fn holders(state: &AppState, role: &str, plant: i64) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, role, &permissions::plant_scope(plant))
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    if ids.is_empty()
        && let Some(user) = role_tasks::assignee(&state.db, role).await?
    {
        ids.push(user);
    }
    Ok(ids)
}

/// Tells `users` about `subject`; no message when there are none.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell(
    state: &AppState,
    subject: Subject,
    label: &str,
    users: &[i64],
    title: &str,
    body: &str,
) -> Result<bool> {
    if users.is_empty() {
        return Ok(false);
    }
    let mut request = NotifyRequest::new(NOTICE, subject)
        .subject_label(label)
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    for user in users {
        request = request.to_user(*user);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(true)
}
