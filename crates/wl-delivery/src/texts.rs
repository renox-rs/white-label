//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_delivery.title", "Deliveries"),
    ("wl_delivery.none", "None"),
    ("wl_delivery.yes", "Yes"),
    ("wl_delivery.no", "No"),
    ("wl_delivery.empty", "No deliveries yet"),
    (
        "wl_delivery.empty_hint",
        "Deliveries that sales orders, dispatch notes and shipments ask for show here.",
    ),
    ("wl_delivery.runs.title", "Runs"),
    ("wl_delivery.runs.empty", "No runs yet"),
    ("wl_delivery.runs.new", "Plan a run"),
    (
        "wl_delivery.runs.new_description",
        "A run is one driver's trip on one day, with a vehicle.",
    ),
    ("wl_delivery.runs.create", "Plan run"),
    ("wl_delivery.runs.created", "Run planned."),
    ("wl_delivery.driver.title", "My stops"),
    ("wl_delivery.driver.none", "You have no stops today."),
    ("wl_delivery.driver.deliver", "Delivered"),
    ("wl_delivery.driver.fail", "Could not deliver"),
    ("wl_delivery.driver.delivered", "Delivery recorded."),
    ("wl_delivery.driver.failed", "Failed visit recorded."),
    ("wl_delivery.fields.number", "Number"),
    ("wl_delivery.fields.source", "Source"),
    ("wl_delivery.fields.recipient", "Recipient"),
    ("wl_delivery.fields.phone", "Phone"),
    ("wl_delivery.fields.address", "Address"),
    ("wl_delivery.fields.area", "Area"),
    ("wl_delivery.fields.window", "Time window"),
    ("wl_delivery.fields.run", "Run"),
    ("wl_delivery.fields.amount_to_collect", "Amount to collect"),
    ("wl_delivery.fields.pod_required", "Proof required"),
    (
        "wl_delivery.fields.partial_acceptance",
        "Partial acceptance",
    ),
    ("wl_delivery.fields.attempts", "Attempts"),
    ("wl_delivery.fields.instructions", "Instructions"),
    ("wl_delivery.fields.date", "Date"),
    ("wl_delivery.fields.driver", "Driver (user id)"),
    ("wl_delivery.fields.vehicle_type", "Vehicle type"),
    ("wl_delivery.fields.vehicle", "Vehicle (asset id)"),
    (
        "wl_delivery.fields.vehicle_hint",
        "Leave empty for an on-foot rider.",
    ),
    ("wl_delivery.fields.depot", "Depot"),
    ("wl_delivery.fields.sequence", "Stop"),
    ("wl_delivery.fields.status", "Status"),
    ("wl_delivery.actions.assign", "Put on a run"),
    ("wl_delivery.actions.assign_title", "Put on a run"),
    (
        "wl_delivery.actions.assign_description",
        "Choose a planned run. The driver's and the vehicle's documents are checked for the run's date.",
    ),
    ("wl_delivery.actions.unassign", "Take off the run"),
    (
        "wl_delivery.actions.unassign_description",
        "The delivery goes back to pending.",
    ),
    ("wl_delivery.actions.dispatch", "Dispatch"),
    (
        "wl_delivery.actions.dispatch_description",
        "The run leaves now. The driver's and the vehicle's documents are checked for today.",
    ),
    ("wl_delivery.actions.assigned", "Delivery put on the run."),
    ("wl_delivery.actions.unassigned", "Delivery taken off the run."),
    ("wl_delivery.actions.dispatched", "Run dispatched."),
    ("wl_delivery.actions.open_review", "Put under review"),
    ("wl_delivery.actions.open_review_title", "Put under review"),
    (
        "wl_delivery.actions.open_review_description",
        "The recipient or the client denies this delivery. Its cash on delivery is held out of remittances and its billing is held until the review is closed.",
    ),
    ("wl_delivery.actions.open_review_reason", "Why is it disputed?"),
    ("wl_delivery.actions.close_review", "Close the review"),
    ("wl_delivery.actions.close_review_title", "Close the review"),
    (
        "wl_delivery.actions.close_review_description",
        "The delivery stands as recorded. Its cash and billing are released and the ticket is resolved.",
    ),
    ("wl_delivery.actions.close_review_outcome", "Outcome"),
    ("wl_delivery.actions.close_review_note", "Note"),
    ("wl_delivery.actions.review_opened", "Delivery put under review."),
    ("wl_delivery.actions.review_closed", "Review closed."),
    ("wl_delivery.outcome.upheld", "Upheld: the delivery stands"),
    ("wl_delivery.review.title", "Review"),
    ("wl_delivery.review.reason", "Reason"),
    ("wl_delivery.review.ticket", "Ticket"),
    ("wl_delivery.review.frozen", "Cash and billing held"),
    ("wl_delivery.review.note", "Closing note"),
    (
        "wl_delivery.review.ticket_title",
        "Delivery {number} disputed",
    ),
    (
        "wl_delivery.review.already",
        "{number} is already under review: add the new complaint to its ticket.",
    ),
    (
        "wl_delivery.review.already_ticket",
        "{number} is already under review: add the new complaint to ticket {ticket}.",
    ),
    (
        "wl_delivery.review.linked_complaint",
        "Another complaint about this delivery came in on ticket {ticket}: {reason}",
    ),
    ("wl_delivery.actions.mark_lost", "Mark lost"),
    ("wl_delivery.actions.mark_lost_title", "Mark as lost"),
    (
        "wl_delivery.actions.mark_lost_description",
        "The goods are neither delivered nor back. Another manager approves it before the delivery is lost; a claim ticket opens and the source stops billing it.",
    ),
    ("wl_delivery.actions.mark_lost_reason", "Reason"),
    ("wl_delivery.actions.mark_lost_liability", "Who bears the loss"),
    ("wl_delivery.actions.mark_lost_requested", "Asked a second manager to approve the loss."),
    ("wl_delivery.actions.mark_lost_done", "Delivery marked lost."),
    ("wl_delivery.actions.found", "Found"),
    ("wl_delivery.actions.found_title", "Goods found"),
    (
        "wl_delivery.actions.found_description",
        "The goods were scanned or confirmed at a depot. The delivery goes back to attempted.",
    ),
    ("wl_delivery.actions.found_depot", "Depot"),
    ("wl_delivery.actions.found_done", "Delivery found again."),
    ("wl_delivery.actions.traced", "Recorded."),
    ("wl_delivery.liability.driver", "The driver"),
    ("wl_delivery.liability.business", "The business"),
    ("wl_delivery.liability.client", "The client"),
    ("wl_delivery.liability.none", "No one"),
    ("wl_delivery.lost.title", "Lost"),
    ("wl_delivery.lost.reason", "Reason"),
    ("wl_delivery.lost.liability", "Who bears the loss"),
    ("wl_delivery.lost.claim", "Claim ticket"),
    (
        "wl_delivery.lost.untraced",
        "Untraced for {days} days (WRK.RUL.17).",
    ),
    (
        "wl_delivery.lost.traced",
        "the goods were scanned, visited or received after the request",
    ),
    (
        "wl_delivery.lost.claim_title",
        "Claim for lost delivery {number}",
    ),
    (
        "wl_delivery.lost.claim_description",
        "Reason: {reason}. Liability: {liability}.",
    ),
    (
        "wl_delivery.lost.found_comment",
        "Goods found at {depot}.",
    ),
    ("wl_delivery.sections.details", "Details"),
    ("wl_delivery.sections.lines", "Lines"),
    ("wl_delivery.sections.proof", "Proof of delivery"),
    ("wl_delivery.sections.attempts", "Attempts"),
    ("wl_delivery.sections.history", "History"),
    ("wl_delivery.sections.stops", "Stops"),
    ("wl_delivery.lines.description", "Item"),
    ("wl_delivery.lines.quantity", "Ordered"),
    ("wl_delivery.lines.delivered", "Delivered"),
    ("wl_delivery.lines.refused", "Refused"),
    ("wl_delivery.lines.reason", "Refusal reason"),
    ("wl_delivery.lines.photo", "Photo"),
    ("wl_delivery.lines.none", "No lines"),
    ("wl_delivery.proof.recipient", "Received by"),
    ("wl_delivery.proof.methods", "Proof captured"),
    ("wl_delivery.proof.at", "Recorded at"),
    ("wl_delivery.proof.cash_expected", "Cash expected"),
    ("wl_delivery.proof.cash_collected", "Cash collected (AED)"),
    ("wl_delivery.proof.cheque", "Cheque reference"),
    ("wl_delivery.proof.photo", "Photo reference"),
    ("wl_delivery.proof.signature", "Signature reference"),
    ("wl_delivery.proof.otp", "OTP verified"),
    ("wl_delivery.proof.latitude", "Latitude"),
    ("wl_delivery.proof.longitude", "Longitude"),
    ("wl_delivery.proof.notes", "Notes"),
    ("wl_delivery.attempts.number", "Visit"),
    ("wl_delivery.attempts.outcome", "Outcome"),
    ("wl_delivery.attempts.reason", "Reason"),
    ("wl_delivery.attempts.at", "At"),
    ("wl_delivery.attempts.none", "No visits yet"),
    ("wl_delivery.history.change", "Change"),
    ("wl_delivery.history.at", "At"),
    ("wl_delivery.history.reason", "Reason"),
    ("wl_delivery.history.none", "No changes yet"),
    ("wl_delivery.stops.none", "No stops on this run"),
    ("wl_delivery.status.pending", "Pending"),
    ("wl_delivery.status.assigned", "Assigned"),
    ("wl_delivery.status.out_for_delivery", "Out for delivery"),
    ("wl_delivery.status.attempted", "Attempted"),
    ("wl_delivery.status.delivered", "Delivered"),
    ("wl_delivery.status.under_review", "Under review"),
    ("wl_delivery.status.lost", "Lost"),
    ("wl_delivery.status.returned", "Returned"),
    ("wl_delivery.status.discarded", "Discarded"),
    ("wl_delivery.status.cancelled", "Cancelled"),
    ("wl_delivery.run_status.planned", "Planned"),
    ("wl_delivery.run_status.dispatched", "Dispatched"),
    ("wl_delivery.run_status.returned_to_depot", "Back at the depot"),
    ("wl_delivery.run_status.closed", "Closed"),
    ("wl_delivery.run_status.cancelled", "Cancelled"),
    ("wl_delivery.vehicle.van", "Van"),
    ("wl_delivery.vehicle.truck", "Truck"),
    ("wl_delivery.vehicle.motorbike", "Motorbike"),
    ("wl_delivery.vehicle.on_foot", "On foot"),
    ("wl_delivery.outcome.delivered", "Delivered"),
    ("wl_delivery.outcome.failed", "Failed"),
    ("wl_delivery.method.photo", "Photo"),
    ("wl_delivery.method.signature", "Signature"),
    ("wl_delivery.method.otp", "OTP"),
    ("wl_delivery.partial.on", "On"),
    ("wl_delivery.partial.off", "Off: every line in full"),
];

/// The English text for `key`, or `key` itself when the package has none.
pub(crate) fn english(key: &str) -> &str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| text)
}

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}

/// Every text of the package in `lang`, keyed without the `wl_delivery.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_delivery."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_delivery.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}
