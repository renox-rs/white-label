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
#[expect(dead_code, reason = "the back-office pages (#64 part 7) use it")]
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}
