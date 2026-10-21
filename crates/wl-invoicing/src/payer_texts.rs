//! The texts of the payer bill (wl-invoicing §4 *Payer bill*), in English.
//! They are listed apart from `texts.rs` and read through the same
//! functions; an app changes or translates one by putting the key in its
//! `resources/lang/{locale}.json`.

/// Every payer bill key, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("wl_invoicing.payer_bill.title", "Payer bills"),
    (
        "wl_invoicing.payer_bill.subtitle",
        "What each payer is asked to pay, and what it paid.",
    ),
    ("wl_invoicing.payer_bill.empty", "No payer bills yet"),
    ("wl_invoicing.payer_bill.draft_number", "Draft"),
    ("wl_invoicing.payer_bill.yes", "Yes"),
    ("wl_invoicing.payer_bill.fields.number", "Number"),
    ("wl_invoicing.payer_bill.fields.payer", "Payer"),
    ("wl_invoicing.payer_bill.fields.customer", "Customer"),
    ("wl_invoicing.payer_bill.fields.source", "Source"),
    ("wl_invoicing.payer_bill.fields.agreement", "Payer agreement"),
    ("wl_invoicing.payer_bill.fields.recipient", "Tax invoice to"),
    ("wl_invoicing.payer_bill.fields.reference", "Payer reference"),
    ("wl_invoicing.payer_bill.fields.service_date", "Service date"),
    ("wl_invoicing.payer_bill.fields.resubmissions", "Resubmissions"),
    ("wl_invoicing.payer_bill.fields.claimed", "Claimed"),
    ("wl_invoicing.payer_bill.fields.paid", "Paid"),
    ("wl_invoicing.payer_bill.fields.denied", "Denied"),
    ("wl_invoicing.payer_bill.fields.open", "Open"),
    ("wl_invoicing.payer_bill.fields.status", "Status"),
    (
        "wl_invoicing.payer_bill.fields.not_in_time",
        "Not submitted in time",
    ),
    ("wl_invoicing.payer_bill.fields.close_reason", "Reason"),
    ("wl_invoicing.payer_bill.status.draft", "Draft"),
    ("wl_invoicing.payer_bill.status.issued", "Issued"),
    ("wl_invoicing.payer_bill.status.partially_paid", "Partly paid"),
    ("wl_invoicing.payer_bill.status.denied", "Denied"),
    ("wl_invoicing.payer_bill.status.paid", "Paid"),
    ("wl_invoicing.payer_bill.status.closed", "Closed"),
    ("wl_invoicing.payer_bill.status.voided", "Void"),
    ("wl_invoicing.payer_bill.recipient.customer", "The customer"),
    ("wl_invoicing.payer_bill.recipient.payer", "The payer"),
    ("wl_invoicing.payer_bill.sections.summary", "Summary"),
    ("wl_invoicing.payer_bill.sections.lines", "Lines"),
    ("wl_invoicing.payer_bill.lines.description", "Description"),
    ("wl_invoicing.payer_bill.lines.claimed", "Claimed"),
    ("wl_invoicing.payer_bill.lines.paid", "Paid"),
    ("wl_invoicing.payer_bill.lines.denied", "Denied"),
    ("wl_invoicing.payer_bill.lines.open", "Open"),
    ("wl_invoicing.payer_bill.lines.code", "Payer code"),
    ("wl_invoicing.payer_bill.lines.empty", "No lines"),
    ("wl_invoicing.payer_bill.issue.action", "Issue"),
    ("wl_invoicing.payer_bill.issue.title", "Issue this payer bill?"),
    (
        "wl_invoicing.payer_bill.issue.message",
        "It is sent to the payer and its receivable opens.",
    ),
    ("wl_invoicing.payer_bill.issue.done", "The payer bill is issued."),
    ("wl_invoicing.payer_bill.resubmit.action", "Resubmit"),
    (
        "wl_invoicing.payer_bill.resubmit.title",
        "Send this payer bill again",
    ),
    (
        "wl_invoicing.payer_bill.resubmit.description",
        "Write down what was corrected. The payer's limit and window apply.",
    ),
    ("wl_invoicing.payer_bill.resubmit.submit", "Resubmit"),
    (
        "wl_invoicing.payer_bill.resubmit.corrections",
        "What was corrected",
    ),
    (
        "wl_invoicing.payer_bill.resubmit.done",
        "The payer bill is sent again.",
    ),
    (
        "wl_invoicing.payer_bill.no_deadline",
        "the agreement gives no deadline",
    ),
    (
        "wl_invoicing.payer_bill.errors.rejection_errors",
        "Give at least one error from the channel, each with its code and message.",
    ),
    (
        "wl_invoicing.payer_bill.errors.rejection_after_remittance",
        "A remittance is already recorded on this payer bill: it cannot go back to draft.",
    ),
    (
        "wl_invoicing.payer_bill.reject.action",
        "Record rejection",
    ),
    (
        "wl_invoicing.payer_bill.reject.title",
        "The channel rejected the submission",
    ),
    (
        "wl_invoicing.payer_bill.reject.description",
        "One error per line, as CODE: message. The bill goes back to draft with the same number; this is not a resubmission.",
    ),
    ("wl_invoicing.payer_bill.reject.submit", "Record rejection"),
    ("wl_invoicing.payer_bill.reject.errors", "Channel errors"),
    (
        "wl_invoicing.payer_bill.reject.done",
        "The rejection is recorded and the bill is a draft again.",
    ),
    ("wl_invoicing.payer_bill.sections.errors", "Submission errors"),
    ("wl_invoicing.payer_bill.errors_table.attempt", "Attempt"),
    ("wl_invoicing.payer_bill.errors_table.code", "Code"),
    ("wl_invoicing.payer_bill.errors_table.message", "Message"),
    ("wl_invoicing.payer_bill.errors_table.date", "Date"),
    (
        "wl_invoicing.payer_bill.waits_for_approval",
        "Sent for approval: a second person must approve it before it is recorded.",
    ),
    ("wl_invoicing.payer_bill.fields.credit", "Payer credit balance"),
    ("wl_invoicing.payer_bill.sections.remittances", "Remittances"),
    ("wl_invoicing.payer_bill.sections.recoupments", "Recoupments"),
    ("wl_invoicing.payer_bill.remit.action", "Record remittance"),
    (
        "wl_invoicing.payer_bill.remit.title",
        "Record the payer's remittance advice",
    ),
    (
        "wl_invoicing.payer_bill.remit.description",
        "Enter what the payer paid and denied per line. An amount above what is open waits for a second person.",
    ),
    ("wl_invoicing.payer_bill.remit.submit", "Record remittance"),
    ("wl_invoicing.payer_bill.remit.reference", "Advice reference"),
    ("wl_invoicing.payer_bill.remit.date", "Advice date"),
    (
        "wl_invoicing.payer_bill.remit.date_hint",
        "YYYY-MM-DD; today when blank.",
    ),
    ("wl_invoicing.payer_bill.remit.payment", "Payment (number)"),
    (
        "wl_invoicing.payer_bill.remit.payment_hint",
        "The payment that carried the money, if it is recorded.",
    ),
    ("wl_invoicing.payer_bill.remit.paid", "paid"),
    ("wl_invoicing.payer_bill.remit.denied", "Denied"),
    ("wl_invoicing.payer_bill.remit.code", "Denial code"),
    ("wl_invoicing.payer_bill.remit.reason", "Denial reason"),
    ("wl_invoicing.payer_bill.remit.overpayment", "To credit balance"),
    (
        "wl_invoicing.payer_bill.remit.done",
        "The remittance is recorded.",
    ),
    ("wl_invoicing.payer_bill.recoup.action", "Record recoupment"),
    (
        "wl_invoicing.payer_bill.recoup.title",
        "The payer took money back",
    ),
    (
        "wl_invoicing.payer_bill.recoup.description",
        "Enter the amount taken back per line. A second person must approve it; the lines are then open again as denied.",
    ),
    ("wl_invoicing.payer_bill.recoup.submit", "Record recoupment"),
    ("wl_invoicing.payer_bill.recoup.code", "Recoupment code"),
    ("wl_invoicing.payer_bill.recoup.reason", "Payer's reason"),
    (
        "wl_invoicing.payer_bill.recoup.carrier",
        "Carried by remittance",
    ),
    (
        "wl_invoicing.payer_bill.recoup.carrier_date",
        "Date of that remittance",
    ),
    ("wl_invoicing.payer_bill.recoup.amount", "Recouped"),
    ("wl_invoicing.payer_bill.recoup.from_credit", "From credit balance"),
    (
        "wl_invoicing.payer_bill.recoup.deducted",
        "Deducted from the remittance",
    ),
    (
        "wl_invoicing.payer_bill.recoup.done",
        "The recoupment is recorded.",
    ),
    (
        "wl_invoicing.payer_bill.errors.amount_format",
        "Enter an amount like 400.00.",
    ),
    (
        "wl_invoicing.payer_bill.errors.date_format",
        "Enter the date as YYYY-MM-DD.",
    ),
    (
        "wl_invoicing.payer_bill.errors.payment_format",
        "The payment is a number.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_reference",
        "Give the payer's reference of the advice.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_lines",
        "The advice needs at least one line.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_unmatched",
        "A line of the advice is not a line of this payer bill.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_twice",
        "A line is on the advice twice.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_amount",
        "Each line needs an amount paid or denied above 0.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_denied",
        "A line cannot be denied for more than is open on it.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_code",
        "A denied amount needs the payer's code.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_used",
        "This advice reference is already recorded for the payer.",
    ),
    (
        "wl_invoicing.payer_bill.errors.remit_changed",
        "The payer bill changed since the advice was entered; enter it again.",
    ),
    (
        "wl_invoicing.payer_bill.errors.recoup_code",
        "Give the payer's recoupment code or reason.",
    ),
    (
        "wl_invoicing.payer_bill.errors.recoup_lines",
        "Name the lines and the amounts taken back.",
    ),
    (
        "wl_invoicing.payer_bill.errors.recoup_amount",
        "Each amount taken back must be above 0.",
    ),
    (
        "wl_invoicing.payer_bill.errors.recoup_above_paid",
        "A line cannot be recouped for more than was paid on it.",
    ),
    (
        "wl_invoicing.payer_bill.errors.recoup_changed",
        "The payer bill changed since the recoupment was entered; enter it again.",
    ),
    (
        "wl_invoicing.payer_bill.errors.agreement_inactive",
        "The payer agreement is not active.",
    ),
    (
        "wl_invoicing.payer_bill.errors.reference",
        "The payer agreement needs a payer reference.",
    ),
    (
        "wl_invoicing.payer_bill.errors.lines",
        "Every line needs a claimed amount above 0.",
    ),
    (
        "wl_invoicing.payer_bill.errors.entity_inactive",
        "The issuing legal entity is not active.",
    ),
    (
        "wl_invoicing.payer_bill.errors.entity_not_registered",
        "A standard-rated line needs an issuing entity that is VAT registered with a TRN.",
    ),
    (
        "wl_invoicing.payer_bill.errors.payer_not_found",
        "The payer was not found.",
    ),
    (
        "wl_invoicing.payer_bill.errors.corrections",
        "Write down what was corrected.",
    ),
    (
        "wl_invoicing.payer_bill.errors.resubmit_window",
        "The payer's resubmission window has passed.",
    ),
    (
        "wl_invoicing.payer_bill.errors.resubmit_limit",
        "The payer's resubmission limit is reached.",
    ),
    (
        "wl_invoicing.payer_bill.errors.already_billed",
        "This source is already billed to that payer agreement.",
    ),
    (
        "wl_invoicing.payer_bill.errors.not_draft",
        "Only a draft payer bill can be corrected.",
    ),
    (
        "wl_invoicing.payer_bill.errors.no_line",
        "The payer bill has no such line.",
    ),
];
