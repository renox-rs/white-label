//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_invoicing.title", "Invoices"),
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
