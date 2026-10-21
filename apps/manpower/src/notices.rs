//! Notices to the people of a client account: a message to the account
//! manager of a client, or to the client company itself, with a title and a
//! body the caller words (09 Manpower §4.1, §7.1).
//!
//! The account manager is the user named on the contract, else the holders of
//! the role in the client company, else whom the kernel picks for the role
//! ([`wl_kernel::role_tasks::assignee`]). The client company is reached by
//! party: its contact points are wl-notify's. The staff keep the item on their
//! task list when a message gives up, so the reaction is `none`; the
//! reactions `09` sets for the client notices (§7.2, *Notification types*)
//! come with BL-MPW-20.

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject, role_tasks};

use crate::permissions;
use crate::texts::template;

/// The notification type of a notice to the staff of a client account.
pub const NOTICE: &str = "manpower.notice";

/// The notification type of a notice to the client company.
pub const CLIENT_NOTICE: &str = "manpower.client_notice";

/// The staff notification type: a title and a body the caller fills in.
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "manpower")
        .group("deployments")
        .category(Category::Reminder)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manpower.notice.subject")),
            template("manpower.notice.body"),
        )
}

/// The client notification type: a title and a body the caller fills in.
pub fn client_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CLIENT_NOTICE, "manpower")
        .group("deployments")
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manpower.notice.subject")),
            template("manpower.notice.body"),
        )
}

/// Registers the notification types.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
    notify::register_type(client_notice_type());
}

/// The account manager of `client`: `named` when the contract names one, else
/// the holders of the role in the client company, else the one the kernel
/// picks; empty when nobody holds it.
///
/// # Errors
///
/// A database error.
pub(crate) async fn account_managers(
    state: &AppState,
    client: PartyId,
    named: Option<i64>,
) -> Result<Vec<i64>> {
    if let Some(user) = named {
        return Ok(vec![user]);
    }
    let mut ids: Vec<i64> = users_with_role_in(
        &state.db,
        permissions::ACCOUNT_MANAGER_ROLE,
        &permissions::client_scope(client.0),
    )
    .await?
    .iter()
    .map(|user| user.id)
    .collect();
    if ids.is_empty()
        && let Some(user) =
            role_tasks::assignee(&state.db, permissions::ACCOUNT_MANAGER_ROLE).await?
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

/// Tells the client company `client` about `subject`.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_client(
    state: &AppState,
    subject: Subject,
    label: &str,
    client: PartyId,
    title: &str,
    body: &str,
) -> Result<()> {
    let request = NotifyRequest::new(CLIENT_NOTICE, subject)
        .subject_label(label)
        .to_party(client)
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_types_have_an_english_template() {
        for def in [notice_type(), client_notice_type()] {
            assert!(
                def.template_for(NotifyChannel::Email, "en")
                    .is_some_and(|text| text.body.contains("{{ body }}"))
            );
        }
        assert_eq!(notice_type().key, NOTICE);
        assert_eq!(client_notice_type().key, CLIENT_NOTICE);
    }
}
