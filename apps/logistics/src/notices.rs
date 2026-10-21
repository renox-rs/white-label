//! The notices to a shipment's client: one notification type that carries a
//! title and a body the caller words (03 Logistics §7.1: *client notified*).
//! A client's message that gives up needs no reaction of its own; the
//! tracking events keep the record.

use renox::prelude::*;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, NotificationTypeDef, NotifyChannel, NotifyRequest,
};

use crate::shipment::Shipment;
use crate::texts::template;

/// The notification type of a notice to a client.
pub const CLIENT_NOTICE: &str = "logistics.client_notice";

/// The notification type: a title and a body the caller fills in.
pub fn client_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(CLIENT_NOTICE, "logistics")
        .group("shipments")
        .category(Category::Transactional)
        .give_up(&[GiveUpReaction::None])
        .template(
            NotifyChannel::Email,
            "en",
            Some(template("logistics.notice.subject")),
            template("logistics.notice.body"),
        )
}

/// Registers the notification type.
pub(crate) fn register_types() {
    notify::register_type(client_notice_type());
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
