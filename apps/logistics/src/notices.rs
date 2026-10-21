//! The notices to a shipment's client: one notification type that carries a
//! title and a body the caller words (03 Logistics §7.1: *client notified*).
//! A client's message that gives up needs no reaction of its own; the
//! tracking events keep the record.

use renox::auth::permissions::users_with_permission;
use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};

use crate::permissions;
use crate::rates::aed;
use crate::remittance::Remittance;
use crate::shipment::Shipment;
use crate::texts::{english, template};

/// The notification type of a notice to a client.
pub const CLIENT_NOTICE: &str = "logistics.client_notice";

/// The notification type: a title and a body the caller fills in. Client
/// shipment notices (pickup short, re-weigh, failed attempt, lost, found,
/// returned, disposed) are in the *marketing and tracking* group: a notice
/// that gives up does nothing more (§9 *Notification types*).
pub fn client_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CLIENT_NOTICE, "logistics")
        .group(notify::GROUP_TRACKING)
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// The notification type of a request that asks the client to act (address
/// confirmation, an instruction for a damaged parcel, freight documents
/// requested) or tells it of a freight job hold or a free-time alert, owned by
/// the dispatcher.
pub const ACTION_REQUEST_DISPATCHER: &str = "logistics.action_request_dispatcher";
/// The same, owned by the documentation officer (freight documents, holds
/// and free-time alerts).
pub const ACTION_REQUEST_DOCUMENTATION: &str = "logistics.action_request_documentation";
/// The notification type of a COD remittance statement and of an adjustment
/// notice.
pub const STATEMENT_NOTICE: &str = "logistics.statement_notice";

/// Every notification type that goes to a client.
pub const CLIENT_TYPES: [&str; 4] = [
    CLIENT_NOTICE,
    ACTION_REQUEST_DISPATCHER,
    ACTION_REQUEST_DOCUMENTATION,
    STATEMENT_NOTICE,
];

/// A request that asks the client to act, in the *portal and signatures*
/// group: a notice that gives up marks the contact invalid and opens a staff
/// task for the owner of the request.
fn action_request_type(key: &str, owner_role: &str) -> NotificationTypeDef {
    NotificationTypeDef::new(key, "logistics")
        .group(notify::GROUP_PORTAL)
        .staff_role(owner_role)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// The statement notice, in the *invoices* group: a notice that gives up goes
/// to the client's other finance contact, then opens a task for the accountant
/// as the collector role.
pub fn statement_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STATEMENT_NOTICE, "logistics")
        .group(notify::GROUP_INVOICES)
        .staff_role(permissions::ACCOUNTANT_ROLE)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// The notification type of the accountants' notice of a drafted COD
/// remittance.
pub const REMITTANCE_DRAFTED: &str = "logistics.remittance_drafted";

/// The notification type: the draft's number and what is on it.
pub fn remittance_drafted_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REMITTANCE_DRAFTED, "logistics")
        .group("remittances")
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.remittance.notice_subject")),
            template("logistics.remittance.notice_body"),
        )
}

/// The notification type of the accountants' notice that an approved
/// remittance went back to a draft because a shipment on it is under review.
pub const REMITTANCE_REOPENED: &str = "logistics.remittance_reopened";

/// The notification type: the remittance's number and the shipment.
pub fn remittance_reopened_type() -> NotificationTypeDef {
    NotificationTypeDef::new(REMITTANCE_REOPENED, "logistics")
        .group("remittances")
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.remittance.reopened_subject")),
            template("logistics.remittance.reopened_body"),
        )
}

/// The notification type of a notice to a staff user: a title and a body the
/// caller words.
pub const STAFF_NOTICE: &str = "logistics.staff_notice";

/// The notification type: a title and a body the caller fills in.
pub fn staff_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STAFF_NOTICE, "logistics")
        .group("remittances")
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// The notification type of the link that asks the client's known contact to
/// confirm a new remittance bank account (`LOG.ACL.13`).
pub const ACCOUNT_CONFIRM: &str = "logistics.account_confirm";

/// The notification type: the link and the last digits of the new account.
pub fn account_confirm_type() -> NotificationTypeDef {
    NotificationTypeDef::new(ACCOUNT_CONFIRM, "logistics")
        .group(notify::GROUP_PORTAL)
        .staff_role(permissions::ACCOUNTANT_ROLE)
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.account.confirm_subject")),
            template("logistics.account.confirm_body"),
        )
}

/// Registers the notification types.
pub(crate) fn register_types() {
    notify::register_type(staff_notice_type());
    notify::register_type(account_confirm_type());
    notify::register_type(client_notice_type());
    notify::register_type(action_request_type(
        ACTION_REQUEST_DISPATCHER,
        permissions::DISPATCHER_ROLE,
    ));
    notify::register_type(action_request_type(
        ACTION_REQUEST_DOCUMENTATION,
        permissions::DOCUMENTATION_OFFICER_ROLE,
    ));
    notify::register_type(statement_notice_type());
    notify::register_type(remittance_drafted_type());
    notify::register_type(remittance_reopened_type());
}

/// Tells the accountants (the holders of `remittances.prepare`) that the
/// approved remittance `remittance` went back to a draft because
/// `shipment` on it is under review.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn tell_reopened(
    state: &AppState,
    remittance: &Remittance,
    shipment: &Shipment,
) -> Result {
    let mut request = NotifyRequest::new(REMITTANCE_REOPENED, remittance.subject())
        .subject_label(&remittance.number)
        .data(renox::serde_json::json!({
            "number": remittance.number,
            "shipment": shipment.number,
            "link": format!("/remittances/{}", remittance.id),
        }));
    let mut told = false;
    for user in users_with_permission(&state.db, permissions::REMITTANCES_PREPARE).await? {
        request = request.to_user(user.id);
        told = true;
    }
    if told {
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Tells the accountants (the holders of `remittances.prepare`) that the
/// draft COD remittance `remittance` was created, or rebuilt with new
/// figures, and what is on it, with any amount carried to the next draft.
/// Nobody is told when nobody holds the permission.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn tell_accountants(
    state: &AppState,
    remittance: &Remittance,
    created: bool,
) -> Result {
    let mut summary = english(
        "logistics.remittance.notice_summary",
        &[
            ("cod", &aed(remittance.cod_total)),
            ("fees", &aed(remittance.fee_total)),
            ("adjustments", &aed(remittance.adjustment_total)),
            ("net", &aed(remittance.net)),
        ],
    );
    if remittance.carried_forward > 0 {
        summary.push(' ');
        summary.push_str(&english(
            "logistics.remittance.notice_carried",
            &[("carried", &aed(remittance.carried_forward))],
        ));
    }
    let what = english(
        if created {
            "logistics.remittance.notice_created"
        } else {
            "logistics.remittance.notice_rebuilt"
        },
        &[],
    );
    let mut request = NotifyRequest::new(REMITTANCE_DRAFTED, remittance.subject())
        .subject_label(&remittance.number)
        .data(renox::serde_json::json!({
            "number": remittance.number,
            "what": what,
            "summary": summary,
            "link": format!("/remittances/{}", remittance.id),
        }));
    let mut told = false;
    for user in users_with_permission(&state.db, permissions::REMITTANCES_PREPARE).await? {
        request = request.to_user(user.id);
        told = true;
    }
    if told {
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Tells the staff user `user` with `title` and `body`, about `subject`
/// (shown as `label`).
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_user(
    state: &AppState,
    subject: Subject,
    label: &str,
    user: i64,
    title: &str,
    body: &str,
) -> Result {
    let request = NotifyRequest::new(STAFF_NOTICE, subject)
        .subject_label(label)
        .to_user(user)
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the client `party` with `title` and `body`, about `subject` (shown
/// as `label`).
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_party(
    state: &AppState,
    subject: Subject,
    label: &str,
    party: PartyId,
    title: &str,
    body: &str,
) -> Result {
    tell_party_as(state, CLIENT_NOTICE, subject, label, party, title, body).await
}

/// [`tell_party`] with the notification type `type_key` (one of
/// [`CLIENT_TYPES`]): the type names the group that decides what happens
/// when the message gives up.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_party_as(
    state: &AppState,
    type_key: &str,
    subject: Subject,
    label: &str,
    party: PartyId,
    title: &str,
    body: &str,
) -> Result {
    let request = NotifyRequest::new(type_key, subject)
        .subject_label(label)
        .to_party(party)
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the client of `shipment` with a request that asks it to act
/// (address confirmation, an instruction for a damaged parcel): when the
/// message gives up, the dispatcher gets a task.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_client_to_act(
    state: &AppState,
    shipment: &Shipment,
    title: &str,
    body: &str,
) -> Result {
    tell_party_as(
        state,
        ACTION_REQUEST_DISPATCHER,
        shipment.subject(),
        &shipment.number,
        shipment.client(),
        title,
        body,
    )
    .await
}

/// Tells the client of `shipment` with `title` and `body`.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn tell_client(
    state: &AppState,
    shipment: &Shipment,
    title: &str,
    body: &str,
) -> Result {
    let request = NotifyRequest::new(CLIENT_NOTICE, shipment.subject())
        .subject_label(&shipment.number)
        .to_party(shipment.client())
        .data(renox::serde_json::json!({ "title": title, "body": body }));
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
