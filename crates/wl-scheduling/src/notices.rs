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

/// One message to a party about every booking of theirs that changed in one
/// action (`bulk_resolve`, the escalation of `SCH.RUL.06`): the resource that
/// took over or a plain change, with *accept*, *pick another time* and
/// *cancel* links for each booking. It never says why.
pub const CHANGED: &str = "scheduling.bookings_changed";

/// One message to a party offering other times for the bookings staff cannot
/// keep (`bulk_resolve` with the action `reschedule`).
pub const RESCHEDULE_OFFER: &str = "scheduling.reschedule_offer";

/// The notification type `scheduling.bookings_changed`.
pub fn changed_type() -> NotificationTypeDef {
    let subject = english("wl_scheduling.notices.changed_subject", &[]);
    let body = english("wl_scheduling.notices.changed_body", &[]);
    NotificationTypeDef::new(CHANGED, "scheduling")
        .group("scheduling")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// The notification type `scheduling.reschedule_offer`.
pub fn reschedule_offer_type() -> NotificationTypeDef {
    let subject = english("wl_scheduling.notices.offer_subject", &[]);
    let body = english("wl_scheduling.notices.offer_body", &[]);
    NotificationTypeDef::new(RESCHEDULE_OFFER, "scheduling")
        .group("scheduling")
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the module's notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(reassigned_type());
    notify::register_type(changed_type());
    notify::register_type(reschedule_offer_type());
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

/// One booking in a message about several.
pub(crate) struct ChangeEntry {
    /// The booking.
    pub booking: Booking,
    /// The resource that took over, when the booking was reassigned.
    pub resource: Option<String>,
    /// The times the party can move to (not held).
    pub slots: Vec<DateTime>,
}

/// Tells `party` about `entries` in one message of type `type_key`
/// ([`CHANGED`] or [`RESCHEDULE_OFFER`]): each booking with its three links.
/// `reason_code` is only in the *cancel* link, never in the text.
pub(crate) async fn changes(
    state: &AppState,
    party: wl_kernel::PartyId,
    entries: &[ChangeEntry],
    type_key: &str,
    reason_code: &str,
    at: DateTime,
) -> Result {
    let Some(first) = entries.first() else {
        return Ok(());
    };
    let mut details = Vec::new();
    for entry in entries {
        let booking = &entry.booking;
        let ttl = link_lifetime(booking, at);
        let when = dubai_text(booking.starts_at);
        let mut lines = vec![match &entry.resource {
            Some(resource) => english(
                "wl_scheduling.notices.line_with",
                &[
                    ("number", &booking.number),
                    ("when", &when),
                    ("resource", resource),
                ],
            ),
            None => english(
                "wl_scheduling.notices.line_change",
                &[("number", &booking.number), ("when", &when)],
            ),
        }];
        if !entry.slots.is_empty() {
            let list = entry
                .slots
                .iter()
                .map(|slot| dubai_text(*slot))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(english("wl_scheduling.notices.line_slots", &[("slots", &list)]));
        }
        lines.push(english(
            "wl_scheduling.notices.line_links",
            &[
                ("accept", &links::accept_url(state, booking.id, ttl)?),
                ("pick", &links::reschedule_url(state, booking.id, ttl)?),
                (
                    "cancel",
                    &links::change_cancel_url(state, booking.id, reason_code, ttl)?,
                ),
            ],
        ));
        details.push(lines.join("\n"));
    }
    let ttl = link_lifetime(&first.booking, at);
    let numbers = entries
        .iter()
        .map(|entry| entry.booking.number.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let data = json!({
        "number": numbers,
        "details": details.join("\n\n"),
        "accept_url": links::accept_url(state, first.booking.id, ttl)?,
        "reschedule_url": links::reschedule_url(state, first.booking.id, ttl)?,
        "cancel_url": links::change_cancel_url(state, first.booking.id, reason_code, ttl)?,
    });
    let request = NotifyRequest::new(type_key, first.booking.subject())
        .to_party(party)
        .subject_label(&numbers)
        .data(data);
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}
