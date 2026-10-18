//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_invoicing.title", "Invoices"),
    ("wl_invoicing.empty", "No invoices yet"),
    (
        "wl_invoicing.empty_hint",
        "Invoices drafted for leases, orders and tickets show here.",
    ),
    ("wl_invoicing.draft_number", "Draft"),
    ("wl_invoicing.pending_approval", "Pending approval"),
    ("wl_invoicing.fields.number", "Number"),
    ("wl_invoicing.fields.party", "Party"),
    ("wl_invoicing.fields.entity", "Issued by"),
    ("wl_invoicing.fields.issue_date", "Issue date"),
    ("wl_invoicing.fields.due_date", "Due date"),
    ("wl_invoicing.fields.date_of_supply", "Date of supply"),
    ("wl_invoicing.fields.currency", "Currency"),
    ("wl_invoicing.fields.net_total", "Net"),
    ("wl_invoicing.fields.vat_total", "VAT"),
    ("wl_invoicing.fields.gross_total", "Total"),
    ("wl_invoicing.fields.allocated_total", "Paid and credited"),
    ("wl_invoicing.fields.balance", "Balance"),
    ("wl_invoicing.fields.status", "Status"),
    ("wl_invoicing.fields.name", "Name"),
    ("wl_invoicing.fields.trn", "TRN"),
    ("wl_invoicing.fields.address", "Address"),
    ("wl_invoicing.fields.billing_email", "Billing e-mail"),
    ("wl_invoicing.fields.description", "Description"),
    ("wl_invoicing.fields.quantity", "Quantity"),
    ("wl_invoicing.fields.unit_price", "Unit price"),
    ("wl_invoicing.fields.discount", "Discount"),
    ("wl_invoicing.fields.net_amount", "Net"),
    ("wl_invoicing.fields.vat_category", "VAT category"),
    ("wl_invoicing.fields.vat_amount", "VAT"),
    ("wl_invoicing.fields.gross_amount", "Total"),
    ("wl_invoicing.fields.source", "Source"),
    ("wl_invoicing.fields.amount", "Amount"),
    ("wl_invoicing.fields.created_at", "Created"),
    ("wl_invoicing.status.draft", "Draft"),
    ("wl_invoicing.status.issued", "Issued"),
    ("wl_invoicing.status.partially_paid", "Partially paid"),
    ("wl_invoicing.status.overdue", "Overdue"),
    ("wl_invoicing.status.paid", "Paid"),
    ("wl_invoicing.status.written_off", "Written off"),
    ("wl_invoicing.status.credited", "Credited"),
    ("wl_invoicing.vat_category.standard", "Standard (5%)"),
    ("wl_invoicing.vat_category.zero_rated", "Zero-rated"),
    ("wl_invoicing.vat_category.exempt", "Exempt"),
    ("wl_invoicing.vat_category.out_of_scope", "Out of scope"),
    ("wl_invoicing.vat_category.reverse_charge", "Reverse charge"),
    ("wl_invoicing.vat.title", "VAT summaries"),
    ("wl_invoicing.vat.status.open", "Open"),
    ("wl_invoicing.vat.status.prepared", "Prepared"),
    ("wl_invoicing.vat.status.reviewed", "Reviewed"),
    ("wl_invoicing.vat.status.locked", "Locked"),
    (
        "wl_invoicing.vat.period_running",
        "The tax period has not ended yet.",
    ),
    (
        "wl_invoicing.vat.same_person",
        "A VAT summary is reviewed by someone other than its preparer.",
    ),
    (
        "wl_invoicing.vat.reviewer_unknown",
        "The summary has no preparer, or the reviewer is unknown.",
    ),
    (
        "wl_invoicing.vat.reason_required",
        "Give a reason for sending the summary back.",
    ),
    ("wl_invoicing.sections.issuer", "Issued by"),
    ("wl_invoicing.sections.party", "Billed to"),
    ("wl_invoicing.sections.totals", "Totals"),
    ("wl_invoicing.sections.lines", "Lines"),
    ("wl_invoicing.sections.vat", "VAT by category"),
    ("wl_invoicing.sections.allocations", "Payments and credits"),
    ("wl_invoicing.sections.write_off", "Write-off"),
    ("wl_invoicing.sections.history", "History"),
    ("wl_invoicing.no_lines", "No lines"),
    (
        "wl_invoicing.no_allocations",
        "Nothing paid or credited yet",
    ),
    ("wl_invoicing.no_history", "No changes yet"),
    ("wl_invoicing.history.change", "Change"),
    ("wl_invoicing.history.at", "When"),
    ("wl_invoicing.history.reason", "Reason"),
    ("wl_invoicing.issue.action", "Issue"),
    ("wl_invoicing.issue.done", "The invoice was issued."),
    ("wl_invoicing.issue.confirm_title", "Issue this invoice?"),
    (
        "wl_invoicing.issue.confirm_message",
        "The invoice gets its number and can no longer be changed.",
    ),
    ("wl_invoicing.write_off.action", "Write off"),
    ("wl_invoicing.write_off.title", "Write off the balance"),
    (
        "wl_invoicing.write_off.description",
        "The open balance of {{ balance }} is written off as a bad debt. The tax invoice stays valid: no credit note is issued. Another approver decides the request.",
    ),
    ("wl_invoicing.write_off.submit", "Request the write-off"),
    ("wl_invoicing.write_off.reason_code", "Reason"),
    ("wl_invoicing.write_off.reason_text", "Details"),
    (
        "wl_invoicing.write_off.done",
        "The balance was written off.",
    ),
    (
        "wl_invoicing.write_off.sent",
        "The write-off was sent for approval.",
    ),
    ("wl_invoicing.write_off.amount", "Amount written off"),
    ("wl_invoicing.write_off.reason", "Reason"),
    ("wl_invoicing.write_off.date", "Write-off date"),
    ("wl_invoicing.write_off.notice_on", "Notice sent on"),
    (
        "wl_invoicing.adjustment.summary_required",
        "A proposal needs the VAT summary to put the adjustment on.",
    ),
    (
        "wl_invoicing.adjustment.reason_required",
        "Give a reason for including or excluding the adjustment.",
    ),
    (
        "wl_invoicing.adjustment.summary_closed",
        "The adjustment can only be decided while its VAT summary is open or prepared.",
    ),
    ("wl_invoicing.write_off.relief", "VAT bad-debt relief"),
    ("wl_invoicing.write_off.relief_status", "Relief status"),
    ("wl_invoicing.adjustment_status.pending", "Pending"),
    ("wl_invoicing.adjustment_status.proposed", "Proposed"),
    ("wl_invoicing.adjustment_status.included", "Included"),
    ("wl_invoicing.adjustment_status.excluded", "Excluded"),
    ("wl_invoicing.write_off_reason.insolvent", "Insolvent"),
    ("wl_invoicing.write_off_reason.untraceable", "Untraceable"),
    (
        "wl_invoicing.write_off_reason.uneconomic_to_pursue",
        "Uneconomic to pursue",
    ),
    (
        "wl_invoicing.write_off_reason.settlement_agreed",
        "Settlement agreed",
    ),
    ("wl_invoicing.write_off_reason.other", "Other"),
    (
        "wl_invoicing.write_off.unknown_reason",
        "Choose one of the write-off reasons.",
    ),
    (
        "wl_invoicing.write_off.reason_text_required",
        "This reason needs a text.",
    ),
    (
        "wl_invoicing.write_off_notice.subject",
        "Invoice {{ number }} written off as a bad debt",
    ),
    (
        "wl_invoicing.write_off_notice.greeting",
        "Dear {{ party_name }},",
    ),
    (
        "wl_invoicing.write_off_notice.intro",
        "We write to you about invoice {{ number }}, issued to you on {{ issue_date }}.",
    ),
    (
        "wl_invoicing.write_off_notice.statement",
        "The amount of {{ written_off }} still open on this invoice was written off as a bad debt on {{ write_off_date }}. This is not a credit note: the tax invoice stands as issued.",
    ),
    (
        "wl_invoicing.write_off_notice.closing",
        "If you have any questions about this notice, please reply to this message.",
    ),
    ("wl_invoicing.write_off_notice.number", "Invoice number"),
    ("wl_invoicing.write_off_notice.issue_date", "Issue date"),
    (
        "wl_invoicing.write_off_notice.date_of_supply",
        "Date of supply",
    ),
    (
        "wl_invoicing.write_off_notice.gross_total",
        "Total including VAT",
    ),
    (
        "wl_invoicing.write_off_notice.written_off",
        "Amount written off",
    ),
    (
        "wl_invoicing.write_off_notice.vat_share",
        "VAT in the amount written off",
    ),
    (
        "wl_invoicing.write_off_notice.write_off_date",
        "Write-off date",
    ),
    (
        "wl_invoicing.write_off_notice.off",
        "The customer write-off notice is turned off, so none was sent.",
    ),
    (
        "wl_invoicing.write_off_notice.no_email",
        "The party has no billing e-mail address, so no notice was sent.",
    ),
    (
        "wl_invoicing.write_off_notice.waiting",
        "The notice is being sent.",
    ),
    (
        "wl_invoicing.reminder.before_due.subject",
        "Invoice {{ number }} is due on {{ due_date }}",
    ),
    (
        "wl_invoicing.reminder.before_due.body",
        "Dear {{ party_name }},\n\nThis is a reminder that invoice {{ number }} of {{ issue_date }} is due on {{ due_date }}. The balance of {{ balance }} is still open.\n\nIf you have already paid, please ignore this message.",
    ),
    (
        "wl_invoicing.reminder.after_due.subject",
        "Invoice {{ number }} is overdue",
    ),
    (
        "wl_invoicing.reminder.after_due.body",
        "Dear {{ party_name }},\n\nInvoice {{ number }} of {{ issue_date }} was due on {{ due_date }}, {{ days }} day(s) ago. The balance of {{ balance }} is still open. Please pay it as soon as possible.\n\nIf you have already paid, please ignore this message.",
    ),
    (
        "wl_invoicing.relief.proposed.subject",
        "Bad-debt VAT relief proposed for invoice {{ number }}",
    ),
    (
        "wl_invoicing.relief.proposed.body",
        "Bad-debt VAT relief of {{ amount }} for invoice {{ number }} ({{ party_name }}) was proposed on the VAT summary for {{ period }}. Include or exclude it, with a reason: {{ link }}",
    ),
];

/// The English text for `key`, or `key` itself when the package has none.
/// Notification templates are registered with it: the client changes them
/// on the stored notification type, not in the language files.
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

/// Every text of the package in `lang`, keyed without the `wl_invoicing.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_invoicing."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_invoicing.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }
}
