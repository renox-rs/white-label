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
    (
        "wl_delivery.actions.unassigned",
        "Delivery taken off the run.",
    ),
    ("wl_delivery.actions.dispatched", "Run dispatched."),
    ("wl_delivery.actions.open_review", "Put under review"),
    ("wl_delivery.actions.open_review_title", "Put under review"),
    (
        "wl_delivery.actions.open_review_description",
        "The recipient or the client denies this delivery. Its cash on delivery is held out of remittances and its billing is held until the review is closed.",
    ),
    (
        "wl_delivery.actions.open_review_reason",
        "Why is it disputed?",
    ),
    ("wl_delivery.actions.close_review", "Close the review"),
    ("wl_delivery.actions.close_review_title", "Close the review"),
    (
        "wl_delivery.actions.close_review_description",
        "The delivery stands as recorded. Its cash and billing are released and the ticket is resolved.",
    ),
    ("wl_delivery.actions.close_review_outcome", "Outcome"),
    ("wl_delivery.actions.close_review_note", "Note"),
    (
        "wl_delivery.actions.review_opened",
        "Delivery put under review.",
    ),
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
    (
        "wl_delivery.actions.mark_lost_liability",
        "Who bears the loss",
    ),
    (
        "wl_delivery.actions.mark_lost_requested",
        "Asked a second manager to approve the loss.",
    ),
    (
        "wl_delivery.actions.mark_lost_done",
        "Delivery marked lost.",
    ),
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
    ("wl_delivery.lost.found_comment", "Goods found at {depot}."),
    (
        "wl_delivery.close.cash_not_counted",
        "The cash the driver collected is not counted yet.",
    ),
    (
        "wl_delivery.close.items_open",
        "{count} undelivered delivery or refused line is neither received back nor reported missing.",
    ),
    (
        "wl_delivery.close.stops_open",
        "{count} stop has no outcome yet: mark it delivered or failed first.",
    ),
    (
        "wl_delivery.close.not_back",
        "The run is not back at the depot.",
    ),
    (
        "wl_delivery.close.not_an_item",
        "That is not an item of this run to receive back.",
    ),
    (
        "wl_delivery.close.counted_by_another",
        "The cash is counted by someone other than the run's driver.",
    ),
    (
        "wl_delivery.close.missing_title",
        "Delivery {number}: {item} reported missing",
    ),
    (
        "wl_delivery.close.missing_delivery",
        "The goods of this delivery did not come back with run {run}. They are traced until found or marked lost (WRK.RUL.17).",
    ),
    (
        "wl_delivery.close.missing_line",
        "{quantity} of this refused line did not come back with run {run}: the source writes the goods off.",
    ),
    ("wl_delivery.close.title", "At the depot"),
    ("wl_delivery.close.received", "Received"),
    ("wl_delivery.close.missing", "Missing"),
    ("wl_delivery.close.item", "Item"),
    ("wl_delivery.close.expected", "To account for"),
    ("wl_delivery.close.cash_expected", "Cash to count"),
    ("wl_delivery.close.cash_counted", "Cash counted (AED)"),
    ("wl_delivery.close.cash_saved", "Cash count recorded."),
    ("wl_delivery.close.receipts_saved", "Receipts recorded."),
    ("wl_delivery.close.closed", "Run closed."),
    ("wl_delivery.close.waiting", "The run stays open"),
    ("wl_delivery.close.save", "Save receipts"),
    ("wl_delivery.close.count", "Record the count"),
    ("wl_delivery.close.close", "Close the run"),
    ("wl_delivery.close.return", "Back at the depot"),
    (
        "wl_delivery.close.stops_hint",
        "Every stop needs an outcome first. The cash and the goods are handed over at the depot.",
    ),
    ("wl_delivery.close.returned", "Run back at the depot."),
    ("wl_delivery.close.none", "Nothing to receive back"),
    ("wl_delivery.close.whole_received", "Received"),
    ("wl_delivery.close.whole_missing", "Missing"),
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
    (
        "wl_delivery.run_status.returned_to_depot",
        "Back at the depot",
    ),
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
    (
        "wl_delivery.close.counted_in_payments",
        "The cash is counted in the cash handovers: count it there and the run closes by itself.",
    ),
    (
        "wl_delivery.close.count_in_payments",
        "The driver's cash of this run is counted in the cash handovers; the run closes when it is counted.",
    ),
    ("wl_delivery.absorber.customer", "Customer"),
    ("wl_delivery.reverse.action", "Reverse delivery"),
    ("wl_delivery.reverse.title", "Reverse this delivery"),
    (
        "wl_delivery.reverse.description",
        "For a delivery marked delivered by mistake. Another manager approves it; the proof is kept and marked reversed, the cash is dealt with and the source takes the goods back.",
    ),
    ("wl_delivery.reverse.reason", "Reason"),
    ("wl_delivery.reverse.liability", "Who is responsible"),
    ("wl_delivery.reverse.note", "Note"),
    ("wl_delivery.reverse.comment", "Comment"),
    (
        "wl_delivery.reverse.reason.marked_by_mistake",
        "Marked delivered by mistake",
    ),
    (
        "wl_delivery.reverse.reason.recipient_denies",
        "Recipient denies the delivery",
    ),
    (
        "wl_delivery.reverse.reason.wrong_recipient",
        "Wrong recipient",
    ),
    ("wl_delivery.reverse.cash", "Cash already handed over"),
    ("wl_delivery.reverse.cash.refund", "Refund to payer"),
    ("wl_delivery.reverse.cash.keep", "Keep for next attempt"),
    (
        "wl_delivery.reverse.handed_over",
        "Cash already handed over",
    ),
    (
        "wl_delivery.reverse.request_title",
        "Reversal waiting for approval",
    ),
    ("wl_delivery.reverse.requested_by", "Asked by"),
    ("wl_delivery.reverse.approve", "Approve reversal"),
    ("wl_delivery.reverse.approve_title", "Approve the reversal"),
    (
        "wl_delivery.reverse.approve_description",
        "The delivery is taken back. When the cash was already handed over, choose what happens to it.",
    ),
    ("wl_delivery.reverse.reject", "Reject reversal"),
    ("wl_delivery.reverse.reject_title", "Reject the reversal"),
    (
        "wl_delivery.reverse.reject_description",
        "The delivery stays delivered and its proof is unchanged.",
    ),
    (
        "wl_delivery.reverse.requested",
        "Reversal sent to another manager for approval.",
    ),
    ("wl_delivery.reverse.done", "Delivery reversed."),
    ("wl_delivery.reverse.approved", "Reversal approved."),
    ("wl_delivery.reverse.rejected_done", "Reversal rejected."),
    (
        "wl_delivery.reverse.choose_reason",
        "Choose why the delivery is reversed: marked by mistake, recipient denies or wrong recipient.",
    ),
    (
        "wl_delivery.reverse.choose_liability",
        "Choose who is responsible: driver, business, client or none.",
    ),
    (
        "wl_delivery.reverse.not_the_driver",
        "The driver of the run cannot request that its delivery is reversed.",
    ),
    (
        "wl_delivery.reverse.no_request",
        "No reversal is waiting for approval on this delivery.",
    ),
    (
        "wl_delivery.reverse.choose_cash",
        "The cash was already handed over: choose refund to payer or keep for next attempt.",
    ),
    (
        "wl_delivery.reverse.approval_failed",
        "The approval could not be applied: the request failed.",
    ),
    (
        "wl_delivery.reverse.cannot_refund",
        "A refund to the payer needs the payments module and a recipient party on the delivery.",
    ),
    (
        "wl_delivery.reverse.refund_reason",
        "Delivery {number} was reversed.",
    ),
    (
        "wl_delivery.reverse.follow_up_comment",
        "The delivery was reversed with the driver responsible: left open for the HR follow-up.",
    ),
    ("wl_delivery.reverse.resolved", "Reversed: {reason}"),
    (
        "wl_delivery.reverse.follow_up_title",
        "Follow up the driver's responsibility for {number}",
    ),
    (
        "wl_delivery.reverse.follow_up_description",
        "Delivery reversed: {reason}. Cash recorded: {cash} ({treatment}). Raise the cash shortage or the HR case.",
    ),
    ("wl_delivery.absorber.client", "Client"),
    ("wl_delivery.absorber.business", "Business"),
    ("wl_delivery.absorber.driver", "Driver"),
    ("wl_delivery.cod.reason", "Reason for the difference"),
    ("wl_delivery.cod.difference", "Difference"),
    ("wl_delivery.cod.expires", "Request lapses"),
    ("wl_delivery.cod.absorbed_by", "Absorbed by"),
    ("wl_delivery.cod.comment", "Comment"),
    (
        "wl_delivery.cod.instruction",
        "Instruction of the source's client (who, when, how)",
    ),
    (
        "wl_delivery.cod.request_title",
        "Cash difference waiting for approval",
    ),
    ("wl_delivery.cod.requested_by", "Asked by"),
    ("wl_delivery.cod.approve", "Approve difference"),
    (
        "wl_delivery.cod.approve_title",
        "Approve the cash difference",
    ),
    (
        "wl_delivery.cod.approve_description",
        "The delivery is delivered with the cash collected. Choose who absorbs the difference; leave it empty to take the reason's default.",
    ),
    ("wl_delivery.cod.reject", "Reject difference"),
    ("wl_delivery.cod.reject_title", "Reject the cash difference"),
    (
        "wl_delivery.cod.reject_description",
        "The delivery stays out; the driver collects in full or fails the attempt.",
    ),
    ("wl_delivery.cod.pending_title", "Waiting for approval"),
    (
        "wl_delivery.cod.pending",
        "The cash difference was sent to an approver. If nobody answers in time, or you do not want to wait, record the failed visit.",
    ),
    (
        "wl_delivery.cod.blocked_title",
        "Only a failed visit is left",
    ),
    ("wl_delivery.cod.no_answer", "No answer: fail attempt"),
    (
        "wl_delivery.cod.requested",
        "Waiting for an approver. If nobody answers in {minutes} minutes, record the failed visit.",
    ),
    ("wl_delivery.cod.approved", "Cash difference approved."),
    ("wl_delivery.cod.rejected_done", "Cash difference rejected."),
    (
        "wl_delivery.cod.no_request",
        "No cash difference is waiting for approval on this delivery.",
    ),
    (
        "wl_delivery.cod.lapsed",
        "No approver answered in time. Record the failed visit with the reason Refused to pay.",
    ),
    (
        "wl_delivery.cod.rejected",
        "The approver rejected the difference. Collect the full amount, or record the failed visit with the reason Refused to pay.",
    ),
    (
        "wl_delivery.cod.timed_out",
        "The request lapsed before it was approved.",
    ),
    (
        "wl_delivery.cod.choose_absorber",
        "Choose who absorbs the difference.",
    ),
    (
        "wl_delivery.cod.not_offered",
        "{absorber} cannot absorb a cash difference here.",
    ),
    (
        "wl_delivery.cod.approval_failed",
        "The approval could not be applied: the request failed.",
    ),
    (
        "wl_delivery.cod.needs_approval",
        "The cash collected is short by more than the tolerance: ask for approval, or fail the attempt.",
    ),
    (
        "wl_delivery.cod.needs_no_approval",
        "The cash collected is within the tolerance: deliver without approval.",
    ),
    // Handing stops over to another driver's run (#165).
    (
        "wl_delivery.hand_over.not_dispatched",
        "Only a run that is out hands stops over.",
    ),
    (
        "wl_delivery.hand_over.reason_unknown",
        "Choose why the stops are handed over: breakdown, accident, driver unwell or load balancing.",
    ),
    (
        "wl_delivery.hand_over.no_stops",
        "Choose at least one open stop of the run.",
    ),
    (
        "wl_delivery.hand_over.not_open_stop",
        "{number} is not an open stop of this run.",
    ),
    (
        "wl_delivery.hand_over.same_run",
        "Choose another run to hand the stops over to.",
    ),
    (
        "wl_delivery.hand_over.same_driver",
        "Stops are handed over to another driver, not to the same one.",
    ),
    (
        "wl_delivery.hand_over.to_run_unknown",
        "The receiving run does not exist.",
    ),
    (
        "wl_delivery.hand_over.to_run_closed",
        "The receiving run must be planned or out.",
    ),
    (
        "wl_delivery.hand_over.only_receiver",
        "Only the driver of the receiving run confirms a hand-over.",
    ),
    (
        "wl_delivery.hand_over.already_confirmed",
        "This hand-over is already confirmed.",
    ),
    (
        "wl_delivery.hand_over.parcels_unconfirmed",
        "Confirm each parcel before taking the stops over: {count} left.",
    ),
    (
        "wl_delivery.hand_over.count_cash",
        "Count the cash before taking the stops over.",
    ),
    (
        "wl_delivery.hand_over.negative_cash",
        "The cash counted cannot be negative.",
    ),
    (
        "wl_delivery.hand_over.nothing_left",
        "No stop of this hand-over is still open.",
    ),
    ("wl_delivery.hand_over.reason.breakdown", "Breakdown"),
    ("wl_delivery.hand_over.reason.accident", "Accident"),
    (
        "wl_delivery.hand_over.reason.driver_unwell",
        "Driver unwell",
    ),
    (
        "wl_delivery.hand_over.reason.load_balancing",
        "Load balancing",
    ),
    ("wl_delivery.hand_over.title", "Hand stops over"),
    ("wl_delivery.hand_over.history", "Hand-overs"),
    ("wl_delivery.hand_over.to_run", "Receiving run"),
    ("wl_delivery.hand_over.reason", "Reason"),
    ("wl_delivery.hand_over.note", "Note"),
    ("wl_delivery.hand_over.stops", "Stops to hand over"),
    ("wl_delivery.hand_over.propose", "Hand over"),
    (
        "wl_delivery.hand_over.proposed",
        "Hand-over sent to the receiving driver.",
    ),
    ("wl_delivery.hand_over.confirmed", "Stops taken over."),
    ("wl_delivery.hand_over.confirm", "Take over the stops"),
    (
        "wl_delivery.hand_over.waiting_title",
        "Stops waiting for you",
    ),
    ("wl_delivery.hand_over.cash_counted", "Cash counted (AED)"),
    (
        "wl_delivery.hand_over.cheques",
        "Cheques received (references, separated by commas)",
    ),
    ("wl_delivery.hand_over.parcel", "Parcel scanned or ticked"),
    ("wl_delivery.hand_over.from_run", "From run"),
    (
        "wl_delivery.hand_over.pending",
        "Waiting for the receiving driver",
    ),
    ("wl_delivery.hand_over.done", "Taken over"),
    // Aborting a run (#164).
    (
        "wl_delivery.abort.choose_reason",
        "Choose why the run is aborted.",
    ),
    (
        "wl_delivery.abort.note_for_other",
        "Say what happened: the reason other needs a note.",
    ),
    (
        "wl_delivery.abort.only_the_driver",
        "Only the driver of the run aborts it from the phone.",
    ),
    (
        "wl_delivery.abort.ticket_unknown",
        "Open a breakdown or an accident ticket, or none.",
    ),
    ("wl_delivery.abort.reason.breakdown", "Breakdown"),
    ("wl_delivery.abort.reason.accident", "Accident"),
    ("wl_delivery.abort.reason.driver_unwell", "Driver unwell"),
    ("wl_delivery.abort.reason.weather", "Weather"),
    ("wl_delivery.abort.reason.other", "Other"),
    (
        "wl_delivery.abort.reason.driver_unreachable",
        "Driver unreachable",
    ),
    (
        "wl_delivery.abort.ticket_title.breakdown",
        "Vehicle breakdown on run {run}",
    ),
    (
        "wl_delivery.abort.ticket_title.accident",
        "Vehicle accident on run {run}",
    ),
    ("wl_delivery.abort.title", "Abort the run"),
    (
        "wl_delivery.abort.description",
        "Stops still out fail with the reason run aborted, recipients are told of the delay, and the cash is counted at the depot.",
    ),
    ("wl_delivery.abort.action", "Abort run"),
    ("wl_delivery.abort.reason", "Reason"),
    ("wl_delivery.abort.note", "Note (needed for other)"),
    (
        "wl_delivery.abort.ticket",
        "Open a ticket about the vehicle",
    ),
    ("wl_delivery.abort.ticket_none", "No ticket"),
    ("wl_delivery.abort.ticket_breakdown", "Breakdown ticket"),
    ("wl_delivery.abort.ticket_accident", "Accident ticket"),
    ("wl_delivery.abort.aborted", "Run aborted."),
    (
        "wl_delivery.abort.driver_hint",
        "If the van cannot go on, say why. Stops you could not hand over fail as run aborted.",
    ),
    (
        "wl_delivery.notices.delayed_subject",
        "Your delivery {{ number }} is delayed",
    ),
    (
        "wl_delivery.notices.delayed_body",
        "Your delivery {{ number }} is delayed: the run carrying it could not go on. We will contact you to arrange a new delivery.",
    ),
    (
        "wl_delivery.notices.aborted_subject",
        "Run {{ run }} was aborted",
    ),
    (
        "wl_delivery.notices.aborted_body",
        "Run {{ run }} was aborted: {{ reason }}. Its cash is counted at the depot: {{ link }}",
    ),
    (
        "wl_delivery.notices.disputed_subject",
        "Delivery {{ number }} is under review",
    ),
    (
        "wl_delivery.notices.disputed_body",
        "Delivery {{ number }} was put under review: {{ reason }}. Its cash is held out of remittances and the sources stop billing it: {{ link }}",
    ),
    (
        "wl_delivery.notices.lost_subject",
        "Delivery {{ number }} is lost",
    ),
    (
        "wl_delivery.notices.lost_body",
        "Delivery {{ number }} is marked lost ({{ liability }} liable): {{ reason }}. A claim ticket was opened: {{ link }}",
    ),
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
