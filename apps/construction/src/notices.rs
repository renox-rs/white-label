//! Notices to the people of a project: a message to the users who hold a
//! role in it (the accountant, the project manager), with a title and a body
//! the caller words (02 Construction §7.1, §7.2).
//!
//! Roles are assigned per project (§3.2), so a notice goes to the holders of
//! the role in the project's scope, else to whom the kernel picks for the
//! role ([`wl_kernel::role_tasks::assignee`]). One notification type carries
//! them all; the staff keep the item on their task list when a message gives
//! up, so the reaction is `none`.

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::role_tasks;

use crate::project::Project;
use crate::texts::template;

/// The notification type of a notice to the people of a project.
pub const NOTICE: &str = "construction.notice";

/// The notification type: a title and a body the caller fills in.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "construction")
        .group("projects")
        .category(Category::Reminder)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("construction.notice.subject")),
            template("construction.notice.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
}

/// The users who hold `role` in `project`, else the one the kernel picks for
/// the role; empty when nobody holds it.
///
/// # Errors
///
/// A database error.
pub(crate) async fn holders(state: &AppState, project: &Project, role: &str) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = users_with_role_in(&state.db, role, &project.scope())
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

/// Tells `users` about `subject`; no message when there are none. Returns
/// whether a message was asked for.
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

/// Tells the holders of `role` in `project` about `subject`.
///
/// # Errors
///
/// A database error and the notifier's errors.
pub(crate) async fn tell_role(
    state: &AppState,
    project: &Project,
    subject: Subject,
    role: &str,
    title: &str,
    body: &str,
) -> Result<bool> {
    let users = holders(state, project, role).await?;
    tell(state, subject, &project.code, &users, title, body).await
}

/// An amount in fils as the messages show it: `AED 50,000.00`.
pub(crate) fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let amount = fils.unsigned_abs();
    let digits = (amount / 100).to_string();
    let mut grouped = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", amount % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_is_shown_in_dirhams_with_thousands() {
        assert_eq!(aed(5_000_000), "AED 50,000.00");
        assert_eq!(aed(123), "AED 1.23");
        assert_eq!(aed(0), "AED 0.00");
        assert_eq!(aed(10_000_000_000), "AED 100,000,000.00");
        assert_eq!(aed(-250_050), "-AED 2,500.50");
    }

    #[test]
    fn the_notice_type_has_an_english_template() {
        let def = notice_type();
        assert_eq!(def.key, NOTICE);
        assert!(
            def.template_for(NotifyChannel::Email, "en")
                .is_some_and(|text| text.body.contains("{{ body }}"))
        );
    }
}
