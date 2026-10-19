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
    (
        "wl_payments.refund_failed.subject",
        "Refund {{ refund }} of {{ amount }} failed",
    ),
    (
        "wl_payments.refund_failed.body",
        "Refund {{ refund }} of {{ amount }} to {{ party }} failed: {{ kind }}. Reason: {{ reason }}. See {{ link }}.",
    ),
    (
        "wl_payments.refund_delayed.subject",
        "Your refund is delayed",
    ),
    (
        "wl_payments.refund_delayed.body",
        "Dear customer, your refund of {{ amount }} is delayed; we will contact you.",
    ),
    (
        "wl_payments.refund_method_changed.subject",
        "Your refund will be paid another way",
    ),
    (
        "wl_payments.refund_method_changed.body",
        "Dear customer, your refund of {{ amount }} will now be paid by {{ method }}.",
    ),
    (
        "wl_payments.refund_cancelled.subject",
        "Your refund is cancelled",
    ),
    (
        "wl_payments.refund_cancelled.body",
        "Dear customer, your refund of {{ amount }} is cancelled. The amount stays on your account as credit.",
    ),
    (
        "wl_payments.refund_method_task.subject",
        "{{ text }}",
    ),
    (
        "wl_payments.refund_method_task.body",
        "{{ text }}: the refund failed ({{ reason }}). Pay it by transfer, a cheque in the party's name or cash. See {{ link }}.",
    ),
    ("wl_payments.tasks.open_refund", "Open the refund"),
    ("wl_payments.refunds.payee_details", "Pays to"),
    ("wl_payments.refunds.retries", "Retries"),
    ("wl_payments.refunds.cancel_reason", "Cancelled because"),
    ("wl_payments.refunds.retry.action", "Retry"),
    ("wl_payments.refunds.retry.title", "Retry this refund?"),
    (
        "wl_payments.refunds.retry.message",
        "It is paid again by the same method to the same details. A refund is retried once.",
    ),
    ("wl_payments.refunds.retry.done", "Refund retried."),
    ("wl_payments.refunds.change.action", "Change method"),
    ("wl_payments.refunds.change.title", "Pay this refund another way"),
    (
        "wl_payments.refunds.change.description",
        "The original card through the gateway, cash and a cheque in the party's name are approved at once. New bank details, or a cheque to someone else, need another user's approval.",
    ),
    ("wl_payments.refunds.change.method", "New method"),
    ("wl_payments.refunds.change.details", "Bank details or payee"),
    (
        "wl_payments.refunds.change.details_hint",
        "The IBAN of a transfer, or the name on a cheque that is not the party's.",
    ),
    ("wl_payments.refunds.change.submit", "Change method"),
    ("wl_payments.refunds.change.done", "The method is changed."),
    (
        "wl_payments.refunds.change.method_required",
        "Choose the new method.",
    ),
    ("wl_payments.refunds.cancel.action", "Cancel refund"),
    ("wl_payments.refunds.cancel.title", "Cancel this refund"),
    (
        "wl_payments.refunds.cancel.description",
        "Use it when the party cannot be reached or asks to keep the credit. The amount stays on the party's credit balance.",
    ),
    ("wl_payments.refunds.cancel.reason", "Reason"),
    ("wl_payments.refunds.cancel.submit", "Cancel refund"),
    ("wl_payments.refunds.cancel.done", "Refund cancelled."),
    ("wl_payments.refunds.cancel.required", "Give the reason."),
    ("wl_payments.refunds.title", "Refunds"),
    ("wl_payments.refunds.empty", "No refunds yet"),
    (
        "wl_payments.refunds.empty_hint",
        "A refund shows here once someone asks to pay money back.",
    ),
    ("wl_payments.refunds.heading", "Refund"),
    ("wl_payments.refunds.sections.details", "Refund"),
    ("wl_payments.refunds.sections.attempts", "Attempts"),
    ("wl_payments.refunds.no_attempts", "Nothing tried yet"),
    ("wl_payments.refunds.no_method", "Not chosen"),
    ("wl_payments.refunds.source", "Paid back from"),
    ("wl_payments.refunds.gateway_txn", "Gateway transaction"),
    ("wl_payments.refunds.reason", "Reason for the refund"),
    ("wl_payments.refunds.paid_at", "Paid on"),
    ("wl_payments.refunds.attempt", "Attempt"),
    ("wl_payments.refunds.failure_kind", "Failure"),
    ("wl_payments.refunds.failed_on", "Failed on"),
    ("wl_payments.refunds.status.requested", "Requested"),
    ("wl_payments.refunds.status.approved", "Approved"),
    ("wl_payments.refunds.status.paid", "Paid"),
    ("wl_payments.refunds.status.failed", "Failed"),
    ("wl_payments.refunds.status.rejected", "Rejected"),
    ("wl_payments.refunds.status.cancelled", "Cancelled"),
    ("wl_payments.refunds.outcome.pending", "Pending"),
    ("wl_payments.refunds.outcome.paid", "Paid"),
    ("wl_payments.refunds.outcome.failed", "Failed"),
    ("wl_payments.refunds.source_kind.payment", "Payment"),
    ("wl_payments.refunds.source_kind.credit_note", "Credit note"),
    ("wl_payments.refunds.source_kind.deposit", "Deposit"),
    (
        "wl_payments.refunds.source_kind.party_credit",
        "Party credit",
    ),
    (
        "wl_payments.refunds.source_kind.payer_credit",
        "Payer credit",
    ),
    ("wl_payments.refunds.source_kind.order", "Order"),
    ("wl_payments.refunds.source_kind.booking", "Booking"),
    (
        "wl_payments.refunds.kind.gateway_declined",
        "Declined by the gateway",
    ),
    (
        "wl_payments.refunds.kind.card_unavailable",
        "Card unavailable",
    ),
    (
        "wl_payments.refunds.kind.bank_return",
        "Returned by the bank",
    ),
    ("wl_payments.refunds.fail.action", "Mark as failed"),
    (
        "wl_payments.refunds.fail.title",
        "Mark this refund as failed",
    ),
    (
        "wl_payments.refunds.fail.description",
        "Use it when the bank returned the transfer or the card cannot take the refund. A paid refund's outgoing payment is reversed and the money goes back to the party.",
    ),
    ("wl_payments.refunds.fail.kind", "What happened"),
    ("wl_payments.refunds.fail.reason", "Reason"),
    ("wl_payments.refunds.fail.submit", "Mark as failed"),
    ("wl_payments.refunds.fail.done", "Refund marked as failed."),
    (
        "wl_payments.refunds.fail.kind_required",
        "Choose what happened.",
    ),
    ("wl_payments.deposits.title", "Deposits"),
    ("wl_payments.deposits.empty", "No deposits yet"),
    (
        "wl_payments.deposits.empty_hint",
        "A deposit shows here once money is held for an agreement.",
    ),
    ("wl_payments.deposits.heading", "Deposit"),
    ("wl_payments.deposits.sections.details", "Deposit"),
    ("wl_payments.deposits.source", "Held for"),
    ("wl_payments.deposits.held", "Held amount"),
    ("wl_payments.deposits.refund_method", "Refund method"),
    ("wl_payments.deposits.refund", "Refund"),
    ("wl_payments.deposits.no_refund", "None yet"),
    ("wl_payments.deposits.refund_failed", "Refund failed"),
    ("wl_payments.deposits.open_refund", "Open the refund"),
    ("wl_payments.deposits.refund_state", "Refund"),
    ("wl_payments.deposits.status.held", "Held"),
    (
        "wl_payments.deposits.status.settlement_pending",
        "Settlement pending",
    ),
    ("wl_payments.deposits.status.refunded", "Refunded"),
    ("wl_payments.deposits.status.forfeited", "Forfeited"),
    ("wl_payments.deposits.status.transferred", "Transferred"),
    ("wl_payments.followup.subject", "{{ text }}"),
    (
        "wl_payments.followup.body",
        "{{ text }}: payment {{ number }} was reversed after the party's credit was refunded. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_bounced.subject",
        "Your cheque {{ number }} was returned",
    ),
    (
        "wl_payments.cheque_bounced.body",
        "Dear customer, your cheque {{ number }} for {{ amount }} drawn on {{ bank }} was returned unpaid by the bank (reason: {{ reason }}). {{ open }} is still due. Please contact us to arrange payment.",
    ),
    ("wl_payments.cheque_task.subject", "{{ text }}"),
    (
        "wl_payments.cheque_task.body",
        "{{ text }}. Cheque {{ number }} of {{ party }}, {{ amount }} open. See {{ link }}.",
    ),
    (
        "wl_payments.cheque_presented.subject",
        "Your cheque {{ number }} will be presented again",
    ),
    (
        "wl_payments.cheque_presented.body",
        "Dear customer, as agreed, your cheque {{ number }} for {{ amount }} will be presented to the bank again on {{ date }}. Please make sure the funds are available.",
    ),
    (
        "wl_payments.cheque_notice.subject",
        "Formal notice: returned cheque {{ number }}",
    ),
    (
        "wl_payments.cheque_notice.body",
        "To {{ drawer }}. Cheque {{ number }} of {{ bank }} dated {{ cheque_date }} for {{ amount }} was returned by the bank on {{ returned_on }} (reason: {{ reason }}). The bank paid {{ honoured }}; {{ open }} is still due{{ fee }}. You are asked to pay the amount still due within {{ days }} days of this notice, failing which we will pursue it through a legal case ({{ case_type }}). Contact: {{ contact }}. (Template content pending legal review.)",
    ),
    (
        "wl_payments.cheque_referred.subject",
        "Cheque {{ number }} referred to a legal case",
    ),
    (
        "wl_payments.cheque_referred.body",
        "{{ text }}. Case owner: {{ owner }}. See {{ link }}.",
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
