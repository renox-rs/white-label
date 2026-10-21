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
