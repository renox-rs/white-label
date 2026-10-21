//! The messages the module sends to a party (wl-scheduling §5.5), through the
//! kernel [`Notifier`](wl_kernel::notify::Notifier). Without wl-notify the
//! kernel's fallback puts them in the bell of the users it knows.

use std::time::Duration;

use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::dubai_text;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::booking::Booking;
use crate::links;
use crate::texts::english;

/// The party is told that the booking moved to another resource at the same
/// time (`reassign`): who, with *accept*, *pick another time* and *cancel*
/// links. It never says why.
pub const REASSIGNED: &str = "scheduling.booking_reassigned";

/// The notification type `scheduling.booking_reassigned`.
pub fn reassigned_type() -> NotificationTypeDef {
    let subject = english("wl_scheduling.notices.reassigned_subject", &[]);
    let body = english("wl_scheduling.notices.reassigned_body", &[]);
    NotificationTypeDef::new(REASSIGNED, "scheduling")
        .group("scheduling")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(reassigned_type());
}

/// How long the links of a message about `booking` last: until the booking
/// ends, and at least an hour.
fn link_lifetime(booking: &Booking, at: DateTime) -> Duration {
    let seconds = (booking.ends_at - at).num_seconds().max(3600);
    Duration::from_secs(u64::try_from(seconds).unwrap_or(3600))
}

/// Tells the party of `booking` that it is now with `resource`. The links
/// are the party's way out: *cancel* cancels as the business (`reason_code`
/// is the code of the change) and gives the deposit back in full; the code
/// itself is in the link, not in the text.
pub(crate) async fn reassigned(
    state: &AppState,
    booking: &Booking,
    resource: &str,
    reason_code: &str,
    at: DateTime,
) -> Result {
    let ttl = link_lifetime(booking, at);
    let data = json!({
        "number": booking.number,
        "resource": resource,
        "when": dubai_text(booking.starts_at),
        "accept_url": links::accept_url(state, booking.id, ttl)?,
        "reschedule_url": links::reschedule_url(state, booking.id, ttl)?,
        "cancel_url": links::change_cancel_url(state, booking.id, reason_code, ttl)?,
    });
    let request = NotifyRequest::new(REASSIGNED, booking.subject())
        .to_party(booking.party())
        .subject_label(&booking.number)
        .data(data);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
