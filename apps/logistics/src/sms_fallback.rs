//! The SMS gateway and its manual fallback (03 Logistics §8 *Integration
//! down*; BL-LOG-25).
//!
//! Tracking SMS to recipients go through the kernel's integration queue
//! ([`queue_tracking_sms`]): while the gateway connection ([`SMS_GATEWAY`]) is
//! not `up` they wait and are sent in order when it recovers. When the
//! connection goes `down` ([`on_status_changed`], on the kernel's
//! `IntegrationStatusChanged`), the dispatcher gets one task that lists the
//! recipients on today's runs to call, and the clients of those shipments are
//! told by e-mail. The recipients on today's runs are the shipments that are
//! out for delivery. The connection itself is registered by the host
//! (`logistics_seed`), with the fallback text of [`crate::defaults`].

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::integrations::{IntegrationStatusChanged, Outbound, Submitted, submit};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::defaults::SMS_GATEWAY;
use crate::notices::tell_party;
use crate::permissions::DISPATCHER_ROLE;
use crate::shipment::{Shipment, ShipmentStatus};
use crate::texts::english;

/// Hands a tracking SMS to the gateway: sent when it is up, queued when it is
/// not. `key` is the natural key of the message (a shipment and a milestone),
/// so the same one is never queued twice.
///
/// # Errors
///
/// The kernel's: the connection is not registered, or the database fails.
pub async fn queue_tracking_sms(
    state: &AppState,
    shipment: &Shipment,
    key: &str,
    text: &str,
) -> Result<Submitted> {
    let call = Outbound::new(
        SMS_GATEWAY,
        "tracking_sms",
        renox::serde_json::json!({
            "shipment": shipment.number,
            "phone": shipment.recipient_phone,
            "text": text,
        }),
    )
    .natural_key(key)
    .subject(shipment.subject());
    submit(state, call).await
}

/// The shipments out for delivery: the recipients on today's runs.
///
/// # Errors
///
/// Database errors.
pub async fn recipients_today(db: &Db) -> Result<Vec<Shipment>> {
    Shipment::query()
        .where_(Shipment::STATUS.eq(ShipmentStatus::OutForDelivery))
        .order_by(Shipment::ID)
        .get(db)
        .await
}

/// The fallback of a gateway that went `down`: the dispatcher's task and the
/// clients' e-mail. Other connections and other changes are left alone. The
/// task is found by its key, so a second event for the same outage opens no
/// second task and tells nobody twice. Returns whether it ran.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn on_status_changed(state: &AppState, event: &IntegrationStatusChanged) -> Result<bool> {
    if event.connection != SMS_GATEWAY || event.to != "down" {
        return Ok(false);
    }
    let since = event
        .failed_since
        .map(|at| at.to_string())
        .unwrap_or_default();
    let key = format!("logistics.sms_down:{since}");
    if wl_kernel::tickets::task_creator(state)
        .find_open(state, &key)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let shipments = recipients_today(&state.db).await?;
    let list: Vec<String> = shipments
        .iter()
        .map(|shipment| {
            format!(
                "{}: {} {}",
                shipment.number, shipment.recipient_name, shipment.recipient_phone
            )
        })
        .collect();
    let title = english("logistics.sms.task_title", &[]);
    let description = english(
        "logistics.sms.task_body",
        &[("count", &shipments.len()), ("list", &list.join("\n"))],
    );
    let task = RoleTask::new(
        wl_kernel::Subject::new(wl_kernel::integrations::SUBJECT_TYPE, 0),
        DISPATCHER_ROLE,
        &title,
        &key,
    )
    .description(&description);
    role_tasks::open(state, task).await?;
    let mut told: Vec<PartyId> = Vec::new();
    for shipment in &shipments {
        let client = shipment.client();
        if told.contains(&client) {
            continue;
        }
        told.push(client);
        let mine = shipments
            .iter()
            .filter(|other| other.client() == client)
            .map(|other| other.number.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let notice_title = english("logistics.sms.notice_title", &[]);
        let notice_body = english("logistics.sms.notice_body", &[("shipments", &mine)]);
        tell_party(
            state,
            shipment.subject(),
            &shipment.number,
            client,
            &notice_title,
            &notice_body,
        )
        .await?;
    }
    Ok(true)
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: IntegrationStatusChanged, state: AppState| async move {
            on_status_changed(&state, &event).await.map(|_| ())
        },
    );
}
