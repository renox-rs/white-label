//! Notices to the people of a client account: a message to the account
//! manager of a client, or to the client company itself, with a title and a
//! body the caller words (09 Manpower §4.1, §7.1).
//!
//! The account manager is the user named on the contract, else the holders of
//! the role in the client company, else whom the kernel picks for the role
//! ([`wl_kernel::role_tasks::assignee`]). The client company is reached by
//! party: its contact points are wl-notify's. The staff keep the item on their
//! task list when a message gives up, so the reaction of a staff notice is
//! `none`; the types and reactions 09 sets for the notices to the client and to
//! workers (§7.2, *Notification types*) are defined here (BL-MPW-20).

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest, ProofMethod,
    Urgency,
};
use wl_kernel::{PartyId, Subject, role_tasks};

use crate::permissions;
use crate::seed;
use crate::texts::template;

/// The notification type of a notice to the staff of a client account.
pub const NOTICE: &str = "manpower.notice";

/// The notification type of a notice to the client company.
pub const CLIENT_NOTICE: &str = "manpower.client_notice";

/// The notification type of the notice that cover is not found in time
/// (`MPW.RUL.13`): a deployment notice that is `urgent`.
pub const CLIENT_NOTICE_URGENT: &str = "manpower.client_notice_urgent";

/// The notification type of a statutory notice to the client company: the
/// confirmation of a contract termination and the notice that workers are
/// withdrawn for non-payment (09 §7.2 *Notification types*).
pub const STATUTORY_NOTICE: &str = "manpower.statutory_notice";

/// The notification type of a message to a worker by SMS (09 §7.2): the
/// payslip, a document expiring, a leave decision, a shift change. A message
/// that cannot be sent marks the number invalid and gives HR a task to reach
/// the worker through the site supervisor.
pub const WORKER_MESSAGE: &str = "manpower.worker_message";

/// The staff notification type: a title and a body the caller fills in. It
/// names no group and takes `none` on give-up: staff always also get the in-app
/// notification (wl-notify §5.1).
pub fn notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NOTICE, "manpower")
        .group("")
        .category(Category::Reminder)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manpower.notice.subject")),
            template("manpower.notice.body"),
        )
}

/// The client notification type (deployment notices to the client: activated,
/// extended, suspended, resumed, ended, cancelled, worker absent): no group,
/// the contact is marked invalid and the account manager gets a task.
pub fn client_notice_type() -> NotificationTypeDef {
    client_type(CLIENT_NOTICE, Urgency::Normal)
}

/// The urgent client notification type: cover not found. It also alerts the
/// holders of the task in the app.
pub fn urgent_client_notice_type() -> NotificationTypeDef {
    client_type(CLIENT_NOTICE_URGENT, Urgency::Urgent)
}

fn client_type(key: &str, urgency: Urgency) -> NotificationTypeDef {
    NotificationTypeDef::new(key, "manpower")
        .group("")
        .category(Category::Transactional)
        .urgency(urgency)
        .give_up(&seed::CLIENT_GIVE_UP)
        .staff_role(permissions::ACCOUNT_MANAGER_ROLE)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manpower.notice.subject")),
            template("manpower.notice.body"),
        )
}

/// The statutory notice to the client: group *Statutory notices*, the contact
/// is marked invalid and the statutory-notice task (deliver another way, record
/// the proof) goes to the account manager.
pub fn statutory_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STATUTORY_NOTICE, "manpower")
        .group(seed::GROUP_STATUTORY)
        .category(Category::Transactional)
        .statutory(true)
        .give_up(&seed::STATUTORY_GIVE_UP)
        .staff_role(permissions::ACCOUNT_MANAGER_ROLE)
        .proof_methods(&[
            ProofMethod::Courier,
            ProofMethod::RegisteredMail,
            ProofMethod::HandDelivery,
            ProofMethod::Call,
        ])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("manpower.notice.subject")),
            template("manpower.notice.body"),
        )
}

/// The SMS to a worker: no group, the number is marked invalid and HR gets a
/// task to reach the worker through the site supervisor.
pub fn worker_message_type() -> NotificationTypeDef {
    NotificationTypeDef::new(WORKER_MESSAGE, "manpower")
        .group("")
        .category(Category::Transactional)
        .give_up(&seed::CLIENT_GIVE_UP)
        .staff_role(permissions::HR_ROLE)
        .channels(&[NotifyChannel::Sms])
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            template("manpower.notice.body"),
        )
}

/// Registers the notification types.
pub(crate) fn register_types() {
    notify::register_type(notice_type());
    notify::register_type(client_notice_type());
    notify::register_type(urgent_client_notice_type());
    notify::register_type(statutory_notice_type());
    notify::register_type(worker_message_type());
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

/// The MD: the holders of the role in the client company (or globally), else
/// the one the kernel picks for the role; empty when nobody holds it.
///
/// # Errors
///
/// A database error.
pub(crate) async fn md_users(state: &AppState, client: PartyId) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = users_with_role_in(
        &state.db,
        permissions::MD_ROLE,
        &permissions::client_scope(client.0),
    )
    .await?
    .iter()
    .map(|user| user.id)
    .collect();
    if ids.is_empty()
        && let Some(user) = role_tasks::assignee(&state.db, permissions::MD_ROLE).await?
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
    tell_client_as(state, CLIENT_NOTICE, subject, label, client, title, body).await
}

/// Tells the client company about `subject` with the notification type
/// `type_key` ([`CLIENT_NOTICE_URGENT`], [`STATUTORY_NOTICE`]).
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_client_as(
    state: &AppState,
    type_key: &str,
    subject: Subject,
    label: &str,
    client: PartyId,
    title: &str,
    body: &str,
) -> Result<()> {
    let request = NotifyRequest::new(type_key, subject)
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
