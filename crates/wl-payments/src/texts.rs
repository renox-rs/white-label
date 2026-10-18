//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::collections::BTreeMap;

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    ("wl_payments.title", "Payments"),
    ("wl_payments.empty", "No payments yet"),
    (
        "wl_payments.empty_hint",
        "Record a payment to see it here with its receipt number.",
    ),
    ("wl_payments.new", "Record payment"),
    ("wl_payments.record", "Record"),
    ("wl_payments.cancel", "Cancel"),
    ("wl_payments.recorded", "Payment recorded."),
    ("wl_payments.no_number", "Pending"),
    ("wl_payments.fields.number", "Number"),
    ("wl_payments.fields.date", "Date"),
    ("wl_payments.fields.party", "Party"),
    ("wl_payments.fields.party_code", "Party code"),
    (
        "wl_payments.fields.party_code_hint",
        "The code of the party, like P-00001.",
    ),
    ("wl_payments.fields.entity", "Legal entity"),
    ("wl_payments.fields.method", "Method"),
    ("wl_payments.fields.direction", "Direction"),
    ("wl_payments.fields.amount", "Amount"),
    (
        "wl_payments.fields.amount_hint",
        "In dirhams, like 1500.00.",
    ),
    ("wl_payments.fields.currency", "Currency"),
    ("wl_payments.fields.allocated", "Allocated"),
    ("wl_payments.fields.unallocated", "Unallocated"),
    ("wl_payments.fields.status", "Status"),
    ("wl_payments.fields.reference", "Reference"),
    ("wl_payments.fields.source", "Source"),
    ("wl_payments.fields.cheque_number", "Cheque number"),
    ("wl_payments.fields.cheque_bank", "Cheque bank"),
    ("wl_payments.fields.cheque_date", "Cheque date"),
    ("wl_payments.fields.created_at", "Created"),
    ("wl_payments.fields.target", "Applied to"),
    ("wl_payments.fields.invoice_id", "Invoice id"),
    (
        "wl_payments.fields.invoice_id_hint",
        "The id of an open invoice of the same party, as in /invoices/{id}.",
    ),
    ("wl_payments.sections.details", "Payment"),
    ("wl_payments.sections.allocations", "Allocations"),
    ("wl_payments.sections.reversal", "Reversal"),
    ("wl_payments.sections.tasks", "Follow-up tasks"),
    ("wl_payments.sections.history", "History"),
    ("wl_payments.no_allocations", "Nothing allocated yet"),
    ("wl_payments.no_tasks", "No follow-up tasks"),
    ("wl_payments.no_history", "No history yet"),
    ("wl_payments.removed", "Removed"),
    ("wl_payments.active", "Active"),
    ("wl_payments.removed_on", "Removed on"),
    ("wl_payments.removed_reason", "Reason"),
    ("wl_payments.status.pending", "Pending"),
    ("wl_payments.status.received", "Received"),
    ("wl_payments.status.failed", "Failed"),
    ("wl_payments.status.charged_back", "Charged back"),
    ("wl_payments.status.reversed", "Reversed"),
    ("wl_payments.method.cash", "Cash"),
    ("wl_payments.method.card", "Card"),
    ("wl_payments.method.transfer", "Bank transfer"),
    ("wl_payments.method.cheque", "Cheque"),
    ("wl_payments.method.gateway", "Online"),
    ("wl_payments.method.direct_debit", "Direct debit"),
    ("wl_payments.method.wallet", "Wallet"),
    ("wl_payments.method.set_off", "Set-off"),
    ("wl_payments.direction.in", "Receipt"),
    ("wl_payments.direction.out", "Payment voucher"),
    ("wl_payments.source.manual", "Entered by a user"),
    ("wl_payments.source.cheque", "Cheque register"),
    ("wl_payments.source.gateway", "Gateway"),
    ("wl_payments.source.bank_statement", "Bank statement"),
    ("wl_payments.source.system", "System"),
    ("wl_payments.reason.entered_in_error", "Entered in error"),
    ("wl_payments.reason.bank_return", "Returned by the bank"),
    ("wl_payments.reason.chargeback", "Chargeback"),
    ("wl_payments.reverse.action", "Reverse"),
    ("wl_payments.reverse.title", "Reverse this payment"),
    (
        "wl_payments.reverse.description",
        "The payment is marked reversed and its allocations are removed (they stay on the page). Invoices are reopened by the invoicing module.",
    ),
    ("wl_payments.reverse.reason", "Reason"),
    ("wl_payments.reverse.note", "Note"),
    ("wl_payments.reverse.submit", "Reverse payment"),
    ("wl_payments.reverse.done", "Payment reversed."),
    ("wl_payments.reverse.reason_required", "Give a reason."),
    ("wl_payments.sections.written_off", "Written-off invoices"),
    ("wl_payments.fields.invoice", "Invoice"),
    ("wl_payments.written_off.left", "Left to recover"),
    ("wl_payments.written_off.proposal", "Proposed amount"),
    (
        "wl_payments.written_off.description",
        "Recovers part of a written-off invoice. What is not allocated stays on the payment as credit.",
    ),
    ("wl_payments.allocate.action", "Allocate"),
    ("wl_payments.allocate.title", "Allocate to an invoice"),
    ("wl_payments.allocate.description", "Left to allocate"),
    ("wl_payments.allocate.submit", "Allocate"),
    ("wl_payments.allocate.done", "Payment allocated."),
    ("wl_payments.tasks.title", "Follow-up tasks"),
    ("wl_payments.tasks.empty", "No open tasks"),
    (
        "wl_payments.tasks.empty_hint",
        "A task shows here when a reversed payment's credit was already refunded.",
    ),
    ("wl_payments.tasks.task", "Task"),
    ("wl_payments.tasks.done_action", "Mark done"),
    ("wl_payments.tasks.done_title", "Mark this task done?"),
    (
        "wl_payments.tasks.done_message",
        "Do this once the charge is raised.",
    ),
    ("wl_payments.tasks.done", "Task marked done."),
    ("wl_payments.tasks.open", "Open"),
    ("wl_payments.tasks.finished", "Done"),
    ("wl_payments.tasks.link", "Open tasks"),
    ("wl_payments.history.change", "Change"),
    ("wl_payments.history.at", "When"),
    ("wl_payments.history.reason", "Reason"),
    ("wl_payments.unknown_party", "No party has this code."),
    ("wl_payments.unknown_entity", "Choose a legal entity."),
    (
        "wl_payments.bad_amount",
        "Enter an amount above 0, like 1500.00.",
    ),
    (
        "wl_payments.returned.subject",
        "Your payment {{ number }} was returned",
    ),
    (
        "wl_payments.returned.body",
        "Your payment {{ number }} of {{ amount }}, made on {{ payment_date }}, was returned by the bank. It no longer counts against your account.",
    ),
    ("wl_payments.followup.subject", "{{ text }}"),
    (
        "wl_payments.followup.body",
        "{{ text }}: payment {{ number }} was reversed after the party's credit was refunded. See {{ link }}.",
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

/// Every text of the package in `lang`, keyed without the `wl_payments.`
/// prefix (`fields.number`), for the templates: `tx['fields.number']`.
pub(crate) fn all(lang: &Lang) -> BTreeMap<&'static str, String> {
    TEXTS
        .iter()
        .map(|(key, _)| (key.trim_start_matches("wl_payments."), t(lang, key)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TEXTS;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS.iter().map(|(key, _)| *key).collect();
        assert!(keys.iter().all(|key| key.starts_with("wl_payments.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}
