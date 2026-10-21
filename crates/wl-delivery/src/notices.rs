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

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(delayed_type());
    notify::register_type(aborted_type());
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
    let mut request = NotifyRequest::new(RUN_ABORTED, run.subject())
        .subject_label(&run.number)
        .data(json!({
            "run": run.number,
            "reason": reason,
            "link": format!("/runs/{}", run.id),
        }));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_types_have_english_templates() {
        for def in [delayed_type(), aborted_type()] {
            assert!(def.template_for(NotifyChannel::Email, "en").is_some());
        }
        assert!(
            delayed_type()
                .template_for(NotifyChannel::Sms, "en")
                .is_some()
        );
    }
}
