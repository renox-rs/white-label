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
    (
        "wl_scheduling.bookings.subtitle",
        "Appointments and slots, newest first.",
    ),
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
    (
        "wl_scheduling.reason.party_request",
        "The party asked to cancel",
    ),
    (
        "wl_scheduling.reason.walked_out",
        "The party left after check-in",
    ),
    ("wl_scheduling.reason.unpaid", "The prepayment was not made"),
    (
        "wl_scheduling.reason.resource_unavailable",
        "The resource is unavailable",
    ),
    (
        "wl_scheduling.reason.resource_ineligible",
        "The resource may not deliver the service",
    ),
    ("wl_scheduling.reason.site_closed", "The site is closed"),
    (
        "wl_scheduling.reason.equipment_failure",
        "Equipment failure",
    ),
    (
        "wl_scheduling.reason.service_not_suitable",
        "The service is not suitable",
    ),
    ("wl_scheduling.reason.other", "Other"),
    ("wl_scheduling.problem.reason_needed", "Give a reason."),
    (
        "wl_scheduling.problem.no_check_in",
        "This service does not need a check-in.",
    ),
    (
        "wl_scheduling.problem.no_in_progress",
        "This service is not tracked while it runs.",
    ),
    (
        "wl_scheduling.problem.not_started",
        "The booking has not started yet.",
    ),
    (
        "wl_scheduling.problem.check_in_first",
        "Check the party in first.",
    ),
    (
        "wl_scheduling.problem.grace_not_over",
        "The grace period after the start has not passed.",
    ),
    (
        "wl_scheduling.problem.conflict_open",
        "The booking has an open conflict; resolve it first.",
    ),
    (
        "wl_scheduling.problem.after_cutoff",
        "It is too late to change this booking yourself; ask the clinic.",
    ),
    ("wl_scheduling.problem.taken", "That slot was just taken."),
    (
        "wl_scheduling.problem.nearest",
        "The nearest free times are :slots.",
    ),
    (
        "wl_scheduling.problem.waiting_list",
        "You can join the waiting list.",
    ),
    (
        "wl_scheduling.problem.closed",
        ":name is not open at that time.",
    ),
    (
        "wl_scheduling.problem.blocked",
        ":name is not available at that time.",
    ),
    (
        "wl_scheduling.problem.no_resource",
        "Choose who or what to book.",
    ),
    (
        "wl_scheduling.problem.unknown_resource",
        "That resource cannot be booked.",
    ),
    (
        "wl_scheduling.problem.not_offered",
        "That resource does not deliver this service.",
    ),
    (
        "wl_scheduling.problem.wrong_resources",
        "Choose one resource of each type the service needs.",
    ),
    (
        "wl_scheduling.problem.unknown_service",
        "That service cannot be booked.",
    ),
    (
        "wl_scheduling.problem.no_request_needed",
        "This service needs no request; book it.",
    ),
    (
        "wl_scheduling.problem.request_needed",
        "This service needs the clinic to accept it; send a request.",
    ),
    (
        "wl_scheduling.problem.not_self_bookable",
        "This service cannot be booked online.",
    ),
    ("wl_scheduling.problem.in_the_past", "That time has passed."),
    (
        "wl_scheduling.problem.too_soon",
        "That is too soon to book online; call the clinic.",
    ),
    (
        "wl_scheduling.problem.unknown_reason",
        "That is not a cancellation reason.",
    ),
    (
        "wl_scheduling.problem.walked_out_early",
        "A party can only walk out after check-in.",
    ),
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
    (
        "wl_scheduling.problem.work_done_pending",
        "Record the work done first, or mark that none was done.",
    ),
    (
        "wl_scheduling.problem.staff_only",
        "Only staff can do this.",
    ),
    (
        "wl_scheduling.problem.not_on_booking",
        "That resource is not on this booking.",
    ),
    (
        "wl_scheduling.problem.already_on_booking",
        "That resource is already on this booking.",
    ),
    ("wl_scheduling.conflicts.title", "Conflicts"),
    (
        "wl_scheduling.conflicts.subtitle",
        "Bookings that a block or an expired licence puts at risk. Nothing is cancelled until you act.",
    ),
    ("wl_scheduling.conflicts.empty", "No bookings in conflict"),
    ("wl_scheduling.conflicts.resource", "Resource"),
    ("wl_scheduling.conflicts.all", "All resources"),
    ("wl_scheduling.conflicts.any", "The first that can take it"),
    ("wl_scheduling.conflicts.day", "Day"),
    ("wl_scheduling.conflicts.filter", "Filter"),
    ("wl_scheduling.conflicts.bookings", "Bookings"),
    ("wl_scheduling.conflicts.action", "Action"),
    ("wl_scheduling.conflicts.to", "Reassign to"),
    ("wl_scheduling.conflicts.reason_code", "Reason code (cancel)"),
    ("wl_scheduling.conflicts.apply", "Apply to the selected"),
    ("wl_scheduling.conflicts.action_reassign", "Reassign to another resource"),
    (
        "wl_scheduling.conflicts.action_reschedule",
        "Offer the parties other times",
    ),
    (
        "wl_scheduling.conflicts.action_cancel",
        "Cancel (the business cancels)",
    ),
    (
        "wl_scheduling.conflicts.result",
        "Done for :done booking(s); failed for :failed.",
    ),
    (
        "wl_scheduling.conflicts.blocked",
        "Blocked. :count booking(s) are in conflict.",
    ),
    (
        "wl_scheduling.conflicts.unblocked",
        "Block removed. :count conflict(s) cleared.",
    ),
    (
        "wl_scheduling.problem.block_period",
        "The block must end after it starts.",
    ),
    (
        "wl_scheduling.problem.unknown_booking",
        "That booking does not exist.",
    ),
    (
        "wl_scheduling.problem.not_conflicted",
        "That booking has no open conflict.",
    ),
    (
        "wl_scheduling.problem.no_replacement",
        "No other resource can take this booking at that time.",
    ),
    (
        "wl_scheduling.notices.reassigned_subject",
        "Your booking {{ number }} has a new resource",
    ),
    (
        "wl_scheduling.notices.reassigned_body",
        "Your booking {{ number }} on {{ when }} is now with {{ resource }}.\n\nKeep it: {{ accept_url }}\nPick another time: {{ reschedule_url }}\nCancel it: {{ cancel_url }}",
    ),
    (
        "wl_scheduling.notices.changed_subject",
        "Your booking {{ number }} has changed",
    ),
    (
        "wl_scheduling.notices.changed_body",
        "Your booking has changed.\n\n{{ details }}",
    ),
    (
        "wl_scheduling.notices.offer_subject",
        "Choose another time for your booking {{ number }}",
    ),
    (
        "wl_scheduling.notices.offer_body",
        "We cannot keep your booking as it is. Please choose another time.\n\n{{ details }}",
    ),
    (
        "wl_scheduling.notices.line_with",
        "Booking :number on :when is now with :resource.",
    ),
    (
        "wl_scheduling.notices.line_change",
        "Booking :number on :when needs to change.",
    ),
    (
        "wl_scheduling.notices.line_slots",
        "You can move it to: :slots.",
    ),
    (
        "wl_scheduling.notices.line_links",
        "Keep it: :accept\nPick another time: :pick\nCancel it: :cancel",
    ),
    (
        "wl_scheduling.notices.escalated_subject",
        "Booking {{ number }} is still in conflict",
    ),
    (
        "wl_scheduling.notices.escalated_body",
        "Booking {{ number }} on {{ when }} with {{ resource }} is still in conflict and starts soon. Reassign it, offer the party another time or cancel it from the list of conflicts.",
    ),
    (
        "wl_scheduling.notices.no_show_subject",
        "Booking {{ number }} was marked no-show",
    ),
    (
        "wl_scheduling.notices.no_show_body",
        "Booking {{ number }} on {{ when }} with {{ resource }} was marked no-show; its slot is free again.",
    ),
    (
        "wl_scheduling.notices.reminder_subject",
        "Reminder: your booking {{ number }}",
    ),
    (
        "wl_scheduling.notices.reminder_body",
        "This is a reminder of your booking {{ number }} on {{ when }} with {{ resource }}.\n\nCancel it: {{ cancel_url }}",
    ),
    (
        "wl_scheduling.link.accepted",
        "Thank you. Your booking stays as it is.",
    ),
    ("wl_scheduling.link.rescheduled", "Your booking is moved."),
    ("wl_scheduling.link.pick_title", "Pick another time"),
    (
        "wl_scheduling.link.no_slots",
        "No other time is free right now. Call the clinic.",
    ),
    ("wl_scheduling.booking.reassign", "Reassign"),
    ("wl_scheduling.booking.reassigned", "Booking reassigned."),
    ("wl_scheduling.booking.reassign_from", "Replace"),
    ("wl_scheduling.booking.reassign_to", "With"),
    (
        "wl_scheduling.problem.nothing_to_waive",
        "There is no forfeited deposit to waive.",
    ),
    ("wl_scheduling.deposit.title", "Deposit"),
    ("wl_scheduling.deposit.held", "held"),
    ("wl_scheduling.deposit.refunded", "refunded"),
    ("wl_scheduling.deposit.forfeited", "forfeited"),
    (
        "wl_scheduling.deposit.applied_to_work_done",
        "applied to the work done",
    ),
    (
        "wl_scheduling.deposit.applied_part",
        ":amount applied to the work done",
    ),
    ("wl_scheduling.booking.work_done", "Work done"),
    ("wl_scheduling.deposit.waive", "Waive forfeiture"),
    (
        "wl_scheduling.deposit.waived",
        "The forfeiture is waived and the deposit is refunded.",
    ),
    (
        "wl_scheduling.deposit.refund_cancelled",
        "Deposit of booking :number paid back.",
    ),
    (
        "wl_scheduling.deposit.refund_waived",
        "Deposit of booking :number paid back: the forfeiture was waived.",
    ),
    (
        "wl_scheduling.waiting.joined",
        "You are on the waiting list.",
    ),
    (
        "wl_scheduling.waiting.withdrawn",
        "You are off the waiting list.",
    ),
    ("wl_scheduling.link.title", "Your booking"),
    ("wl_scheduling.link.cancel", "Cancel this booking"),
    ("wl_scheduling.link.cancelled", "The booking is cancelled."),
    (
        "wl_scheduling.problem.offer_mismatch",
        "That slot does not match what the entry waits for.",
    ),
    (
        "wl_scheduling.problem.offer_expired",
        "The offer has expired.",
    ),
    (
        "wl_scheduling.problem.offer_still_held",
        "The offer is still held.",
    ),
    (
        "wl_scheduling.problem.not_expired",
        "The entry has not expired.",
    ),
    (
        "wl_scheduling.problem.choose_resource",
        "Choose who or what to book.",
    ),
    (
        "wl_scheduling.problem.too_far",
        "That is further ahead than bookings are taken.",
    ),
    (
        "wl_scheduling.problem.not_available",
        "That time cannot be booked. Please choose another.",
    ),
    (
        "wl_scheduling.problem.payment_unavailable",
        "The payment for that time cannot be taken now. Please try again later.",
    ),
    (
        "wl_scheduling.prepayment.refund_reason",
        "slot not available",
    ),
    (
        "wl_scheduling.prepayment.task_title",
        "Refund the late payment for booking :number",
    ),
    (
        "wl_scheduling.prepayment.task_body",
        "Payment :payment arrived after booking :number was released and the refund could not be asked automatically. Refund it in full from the payment.",
    ),
    (
        "wl_scheduling.notices.unpaid_subject",
        "Your booking {{ number }} was not held",
    ),
    (
        "wl_scheduling.notices.unpaid_body",
        "Your booking {{ number }} on {{ when }} was released because the payment did not arrive in time. Please book again.",
    ),
    (
        "wl_scheduling.notices.late_payment_subject",
        "Your payment for booking {{ number }} is being refunded",
    ),
    (
        "wl_scheduling.notices.late_payment_body",
        "Your payment arrived after booking {{ number }} on {{ when }} was released, so the time is no longer held. We are refunding it in full. Please book again.",
    ),
    (
        "wl_scheduling.eligibility.expired",
        ":item expired on :date",
    ),
    ("wl_scheduling.eligibility.missing", ":item is missing"),
    (
        "wl_scheduling.eligibility.not_valid",
        ":item is not valid on :date",
    ),
    (
        "wl_scheduling.eligibility.hint_reassign",
        "reassign first",
    ),
    (
        "wl_scheduling.eligibility.hint_rebook",
        "cancel with resource_ineligible and rebook",
    ),
    (
        "wl_scheduling.eligibility.task_title",
        "Reassign or reschedule booking :number: :item",
    ),
    (
        "wl_scheduling.eligibility.task_body",
        "The resource of booking :number on :when will not hold a valid :item then. Reassign the booking to another resource or move it to another time; the party is told only that the booking changed.",
    ),
    (
        "wl_scheduling.eligibility.task_resolved",
        "The item was renewed; the conflict is cleared.",
    ),
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
        assert_eq!(
            english("wl_scheduling.nothing", &[]),
            "wl_scheduling.nothing"
        );
    }
}
