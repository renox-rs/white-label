//! The notices of the module, sent through the kernel
//! [`Notifier`](wl_kernel::notify::Notifier) (transactional, English texts
//! first). Without `wl-notify` the kernel's fallback puts a notice for a
//! staff user in the bell; a recipient who is no staff user gets nothing.
//!
//! - `delivery.run_delayed`: a recipient is told that the run carrying their
//!   delivery was aborted and the delivery will be arranged again (#164,
//!   `WRK.RUL.07`). It goes to the recipient party when the delivery knows
//!   one, else to the phone number of the delivery (SMS); a delivery with
//!   neither is skipped.
//! - `delivery.run_aborted`: the dispatchers (the holders of `runs.dispatch`;
//!   the host has no separate depot manager role) are told that a run was
//!   aborted and why.
//! - `delivery.disputed` and `delivery.lost`: the dispatchers are told that a
//!   delivery was put under review (#160) or is lost (#162), with the
//!   reason, the liability and the ticket. The source of the delivery is
//!   reached through the events (`DeliveryDisputed`, `DeliveryLost` carry
//!   it); the kernel has no contract naming the owner of a source record, so
//!   no notice goes to one here (the sources' own listeners, BL-SAL-13,
//!   BL-LOG-07, tell theirs).

use renox::auth::permissions::users_with_permission;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest};

use crate::delivery::Delivery;
use crate::permissions;
use crate::run::Run;
use crate::texts::english;

/// The recipient is told their delivery is delayed.
pub const RUN_DELAYED: &str = "delivery.run_delayed";
/// The dispatchers are told a run was aborted.
pub const RUN_ABORTED: &str = "delivery.run_aborted";

fn delayed_type() -> NotificationTypeDef {
    let body = english("wl_delivery.notices.delayed_body");
    NotificationTypeDef::new(RUN_DELAYED, "delivery")
        .group("delivery")
        .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
        .template(NotifyChannel::Sms, "en", None, body)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.delayed_subject")),
            body,
        )
}

fn aborted_type() -> NotificationTypeDef {
    NotificationTypeDef::new(RUN_ABORTED, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.aborted_subject")),
            english("wl_delivery.notices.aborted_body"),
        )
}

/// The dispatchers are told a delivery was put under review.
pub const DELIVERY_DISPUTED: &str = "delivery.disputed";
/// The dispatchers are told a delivery is lost.
pub const DELIVERY_LOST: &str = "delivery.lost";

fn disputed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DELIVERY_DISPUTED, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.disputed_subject")),
            english("wl_delivery.notices.disputed_body"),
        )
}

fn lost_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DELIVERY_LOST, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.lost_subject")),
            english("wl_delivery.notices.lost_body"),
        )
}

/// The dispatchers are told a perishable stop failed and what the policy
/// decided (`WRK.RUL.16`).
pub const PERISHABLE_FAILED: &str = "delivery.perishable_failed";
/// The dispatchers are told a delivery was returned to the origin.
pub const DELIVERY_RETURNED: &str = "delivery.returned";
/// The dispatchers are told perishable goods were discarded.
pub const DELIVERY_DISCARDED: &str = "delivery.discarded";

fn perishable_failed_type() -> NotificationTypeDef {
    NotificationTypeDef::new(PERISHABLE_FAILED, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.perishable_failed_subject")),
            english("wl_delivery.notices.perishable_failed_body"),
        )
}

fn returned_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DELIVERY_RETURNED, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.returned_subject")),
            english("wl_delivery.notices.returned_body"),
        )
}

fn discarded_type() -> NotificationTypeDef {
    NotificationTypeDef::new(DELIVERY_DISCARDED, "delivery")
        .group("delivery")
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_delivery.notices.discarded_subject")),
            english("wl_delivery.notices.discarded_body"),
        )
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(perishable_failed_type());
    notify::register_type(returned_type());
    notify::register_type(discarded_type());
    notify::register_type(delayed_type());
    notify::register_type(aborted_type());
    notify::register_type(disputed_type());
    notify::register_type(lost_type());
}

/// Sends `request` to the holders of `runs.dispatch`; nothing when nobody
/// holds it.
async fn to_dispatchers(state: &AppState, mut request: NotifyRequest) -> Result<()> {
    let mut told = false;
    for user in users_with_permission(&state.db, permissions::RUNS_DISPATCH).await? {
        request = request.to_user(user.id);
        told = true;
    }
    if told {
        notify::notifier(state).notify(state, request).await?;
    }
    Ok(())
}

/// Tells the dispatchers that `delivery` was put under review for `reason`.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn disputed(
    state: &AppState,
    delivery: &Delivery,
    reason: &str,
    ticket: Option<i64>,
) -> Result<()> {
    let request = NotifyRequest::new(DELIVERY_DISPUTED, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "reason": reason,
            "ticket": ticket,
            "link": format!("/deliveries/{}", delivery.id),
        }));
    to_dispatchers(state, request).await
}

/// Tells the dispatchers that `delivery` is lost, with the liability and the
/// claim ticket.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn lost(
    state: &AppState,
    delivery: &Delivery,
    liability: &str,
    claim_ticket: Option<i64>,
) -> Result<()> {
    let request = NotifyRequest::new(DELIVERY_LOST, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "reason": delivery.lost_reason.clone().unwrap_or_default(),
            "liability": liability,
            "ticket": claim_ticket,
            "link": format!("/deliveries/{}", delivery.id),
        }));
    to_dispatchers(state, request).await
}

/// Tells the dispatchers (the origin's managers) that the perishable
/// `delivery` failed and what follows: the goods are discarded with a photo
/// (`discard`), or go back to the origin.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn perishable_failed(
    state: &AppState,
    delivery: &Delivery,
    discard: bool,
) -> Result<()> {
    let request = NotifyRequest::new(PERISHABLE_FAILED, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "action": english(if discard {
                "wl_delivery.notices.action_discard"
            } else {
                "wl_delivery.notices.action_return"
            }),
            "link": format!("/deliveries/{}", delivery.id),
        }));
    to_dispatchers(state, request).await
}

/// Tells the dispatchers that `delivery` went back to the origin.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn returned(state: &AppState, delivery: &Delivery) -> Result<()> {
    let request = NotifyRequest::new(DELIVERY_RETURNED, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "link": format!("/deliveries/{}", delivery.id),
        }));
    to_dispatchers(state, request).await
}

/// Tells the dispatchers that the goods of `delivery` were discarded for
/// `reason`.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn discarded(state: &AppState, delivery: &Delivery, reason: &str) -> Result<()> {
    let request = NotifyRequest::new(DELIVERY_DISCARDED, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "reason": reason,
            "link": format!("/deliveries/{}", delivery.id),
        }));
    to_dispatchers(state, request).await
}

/// Tells the recipient of `delivery` that its run was aborted.
///
/// # Errors
///
/// The notifier's errors.
pub(crate) async fn run_delayed(state: &AppState, delivery: &Delivery, run: &Run) -> Result<()> {
    let mut request = NotifyRequest::new(RUN_DELAYED, delivery.delivery_id().subject())
        .subject_label(&delivery.number)
        .data(json!({
            "number": delivery.number,
            "recipient": delivery.recipient_name,
            "run": run.number,
        }));
    if let Some(party) = delivery.recipient() {
        request = request.to_party(party);
    } else if !delivery.recipient_phone.trim().is_empty() {
        request = request.to(NotifyRecipient::Address {
            channel: NotifyChannel::Sms,
            address: delivery.recipient_phone.trim().to_owned(),
        });
    } else {
        return Ok(());
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// Tells the dispatchers that `run` was aborted, with the reason as it reads
/// in the history.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn run_aborted(state: &AppState, run: &Run, reason: &str) -> Result<()> {
    let request = NotifyRequest::new(RUN_ABORTED, run.subject())
        .subject_label(&run.number)
        .data(json!({
            "run": run.number,
            "reason": reason,
            "link": format!("/runs/{}", run.id),
        }));
    to_dispatchers(state, request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_types_have_english_templates() {
        for def in [
            delayed_type(),
            aborted_type(),
            disputed_type(),
            lost_type(),
            perishable_failed_type(),
            returned_type(),
            discarded_type(),
        ] {
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        }
        assert!(
            delayed_type()
                .template_for(NotifyChannel::Sms, "en")
                .is_some()
        );
    }
}
