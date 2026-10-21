//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_scheduling.cancel", "Cancel"),
    ("wl_scheduling.none", "—"),
    ("wl_scheduling.fields.status", "Status"),
    ("wl_scheduling.bookings.title", "Bookings"),
    ("wl_scheduling.bookings.subtitle", "Appointments and slots, newest first."),
    ("wl_scheduling.bookings.empty", "No bookings yet"),
    ("wl_scheduling.bookings.number", "Number"),
    ("wl_scheduling.bookings.party", "Booked by"),
    ("wl_scheduling.bookings.service", "Service"),
    ("wl_scheduling.bookings.when", "Starts"),
    ("wl_scheduling.bookings.resources", "With"),
    ("wl_scheduling.booking.details", "Booking"),
    ("wl_scheduling.booking.ends", "Ends"),
    ("wl_scheduling.booking.source", "Booked through"),
    ("wl_scheduling.booking.notes", "Notes"),
    ("wl_scheduling.booking.cancel_reason", "Cancellation reason"),
    ("wl_scheduling.booking.accept", "Accept"),
    ("wl_scheduling.booking.decline", "Decline"),
    ("wl_scheduling.booking.confirm", "Confirm"),
    ("wl_scheduling.booking.reschedule", "Reschedule"),
    ("wl_scheduling.booking.check_in", "Check in"),
    ("wl_scheduling.booking.start", "Start"),
    ("wl_scheduling.booking.complete", "Complete"),
    ("wl_scheduling.booking.no_show", "Mark no-show"),
    ("wl_scheduling.booking.cancel", "Cancel booking"),
    ("wl_scheduling.booking.reason", "Reason"),
    ("wl_scheduling.booking.reason_code", "Reason code"),
    ("wl_scheduling.booking.side", "Caused by"),
    ("wl_scheduling.booking.side_default", "As the reason says"),
    ("wl_scheduling.booking.side_party", "The party"),
    ("wl_scheduling.booking.side_business", "The business"),
    ("wl_scheduling.booking.starts_at", "New start"),
    ("wl_scheduling.booking.booked", "Booking made."),
    ("wl_scheduling.booking.accepted", "Booking accepted."),
    ("wl_scheduling.booking.declined", "Booking declined."),
    ("wl_scheduling.booking.confirmed", "Booking confirmed."),
    ("wl_scheduling.booking.rescheduled", "Booking moved."),
    ("wl_scheduling.booking.cancelled", "Booking cancelled."),
    ("wl_scheduling.booking.no_show_done", "Marked as a no-show."),
    ("wl_scheduling.booking.completed", "Booking completed."),
    ("wl_scheduling.booking.checked_in", "Party checked in."),
    ("wl_scheduling.booking.started", "Service started."),
    ("wl_scheduling.status.requested", "Requested"),
    ("wl_scheduling.status.booked", "Booked"),
    ("wl_scheduling.status.confirmed", "Confirmed"),
    ("wl_scheduling.status.checked_in", "Checked in"),
    ("wl_scheduling.status.in_progress", "In progress"),
    ("wl_scheduling.status.completed", "Completed"),
    ("wl_scheduling.status.no_show", "No-show"),
    ("wl_scheduling.status.cancelled", "Cancelled"),
    ("wl_scheduling.reason.party_request", "The party asked to cancel"),
    ("wl_scheduling.reason.walked_out", "The party left after check-in"),
    ("wl_scheduling.reason.unpaid", "The prepayment was not made"),
    ("wl_scheduling.reason.resource_unavailable", "The resource is unavailable"),
    ("wl_scheduling.reason.resource_ineligible", "The resource may not deliver the service"),
    ("wl_scheduling.reason.site_closed", "The site is closed"),
    ("wl_scheduling.reason.equipment_failure", "Equipment failure"),
    ("wl_scheduling.reason.service_not_suitable", "The service is not suitable"),
    ("wl_scheduling.reason.other", "Other"),
    ("wl_scheduling.problem.reason_needed", "Give a reason."),
    ("wl_scheduling.problem.no_check_in", "This service does not need a check-in."),
    ("wl_scheduling.problem.no_in_progress", "This service is not tracked while it runs."),
    ("wl_scheduling.problem.not_started", "The booking has not started yet."),
    ("wl_scheduling.problem.check_in_first", "Check the party in first."),
    ("wl_scheduling.problem.grace_not_over", "The grace period after the start has not passed."),
    ("wl_scheduling.problem.conflict_open", "The booking has an open conflict; resolve it first."),
    (
        "wl_scheduling.problem.after_cutoff",
        "It is too late to change this booking yourself; ask the clinic.",
    ),
    ("wl_scheduling.problem.taken", "That slot was just taken."),
    ("wl_scheduling.problem.nearest", "The nearest free times are :slots."),
    ("wl_scheduling.problem.waiting_list", "You can join the waiting list."),
    ("wl_scheduling.problem.closed", ":name is not open at that time."),
    ("wl_scheduling.problem.blocked", ":name is not available at that time."),
    ("wl_scheduling.problem.no_resource", "Choose who or what to book."),
    ("wl_scheduling.problem.unknown_resource", "That resource cannot be booked."),
    ("wl_scheduling.problem.not_offered", "That resource does not deliver this service."),
    (
        "wl_scheduling.problem.wrong_resources",
        "Choose one resource of each type the service needs.",
    ),
    ("wl_scheduling.problem.unknown_service", "That service cannot be booked."),
    ("wl_scheduling.problem.no_request_needed", "This service needs no request; book it."),
    (
        "wl_scheduling.problem.request_needed",
        "This service needs the clinic to accept it; send a request.",
    ),
    ("wl_scheduling.problem.not_self_bookable", "This service cannot be booked online."),
    ("wl_scheduling.problem.in_the_past", "That time has passed."),
    ("wl_scheduling.problem.too_soon", "That is too soon to book online; call the clinic."),
    ("wl_scheduling.problem.unknown_reason", "That is not a cancellation reason."),
    ("wl_scheduling.problem.walked_out_early", "A party can only walk out after check-in."),
    (
        "wl_scheduling.problem.staff_after_check_in",
        "Only staff can cancel once the party is checked in.",
    ),
    (
        "wl_scheduling.problem.initiated_by_needed",
        "Say whether the party or the business cancelled.",
    ),
    (
        "wl_scheduling.problem.party_cannot_blame",
        "A party cannot record the business as the cause.",
    ),
    (
        "wl_scheduling.problem.override_needs_note",
        "To record the other side as the cause, add a note saying why.",
    ),
    ("wl_scheduling.problem.nothing_to_waive", "There is no forfeited deposit to waive."),
    ("wl_scheduling.deposit.title", "Deposit"),
    ("wl_scheduling.deposit.held", "held"),
    ("wl_scheduling.deposit.refunded", "refunded"),
    ("wl_scheduling.deposit.forfeited", "forfeited"),
    ("wl_scheduling.deposit.applied_to_work_done", "applied to the work done"),
    ("wl_scheduling.deposit.waive", "Waive forfeiture"),
    ("wl_scheduling.deposit.waived", "The forfeiture is waived and the deposit is refunded."),
    (
        "wl_scheduling.deposit.refund_cancelled",
        "Deposit of booking :number paid back.",
    ),
    (
        "wl_scheduling.deposit.refund_waived",
        "Deposit of booking :number paid back: the forfeiture was waived.",
    ),
    ("wl_scheduling.waiting.joined", "You are on the waiting list."),
    ("wl_scheduling.waiting.withdrawn", "You are off the waiting list."),
    ("wl_scheduling.link.title", "Your booking"),
    ("wl_scheduling.link.cancel", "Cancel this booking"),
    ("wl_scheduling.link.cancelled", "The booking is cancelled."),
    (
        "wl_scheduling.problem.offer_mismatch",
        "That slot does not match what the entry waits for.",
    ),
    ("wl_scheduling.problem.offer_expired", "The offer has expired."),
    ("wl_scheduling.problem.offer_still_held", "The offer is still held."),
    ("wl_scheduling.problem.not_expired", "The entry has not expired."),
    (
        "wl_scheduling.problem.choose_resource",
        "Choose who or what to book.",
    ),
    ("wl_scheduling.problem.too_far", "That is further ahead than bookings are taken."),
];

/// The package text `key` in `lang`: the app's translation if it has one,
/// else the English text.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    t_with(lang, key, &[])
}

/// Like [`t`], replacing `:name` placeholders with `params`.
pub(crate) fn t_with(lang: &Lang, key: &str, params: &[(&str, &dyn Display)]) -> String {
    let found = lang.t(key, params);
    if found != key {
        return found;
    }
    english(key, params)
}

/// Every text of the package in `lang`, keyed without the `wl_scheduling.`
/// prefix (`bookings.title`), for the templates: `tx['bookings.title']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_scheduling."), t(lang, key)))
        .collect()
}

/// The English text for `key` with `params` filled in; the key itself when
/// it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &dyn Display)]) -> String {
    let text = TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let params: Vec<(&str, String)> = params.iter().map(|(k, v)| (*k, v.to_string())).collect();
    renox::i18n::format(text, &params, None)
}

#[cfg(test)]
mod tests {
    use super::{TEXTS, english};

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_scheduling.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_missing_key_answers_itself() {
        assert_eq!(english("wl_scheduling.nothing", &[]), "wl_scheduling.nothing");
    }
}
