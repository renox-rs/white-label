//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

use crate::payout_texts::PAYOUT_TEXTS;
use crate::screen_texts::SCREEN_TEXTS;

/// Every key the app uses besides the screens', with its English text.
const TEXTS: &[(&str, &str)] = &[
    // The notices.
    ("fnb.notice.subject", "{{ title }}"),
    ("fnb.notice.body", "{{ body }}"),
    // Toasts of the screens.
    ("fnb.accepted", "Order accepted."),
    ("fnb.rejected", "Order rejected."),
    ("fnb.details_saved", "Delivery details saved."),
    ("fnb.dispatched", "Order dispatched."),
    ("fnb.completed", "Order completed."),
    ("fnb.closed", "The failed delivery is closed."),
    ("fnb.party_changed", "The responsible party is changed."),
    ("fnb.refund_asked", "The refund is asked for. A kitchen manager approves it."),
    ("fnb.ticket_started", "Ticket started."),
    ("fnb.ticket_bumped", "Ticket bumped."),
    ("fnb.cod_on", "Cash on delivery is on again for the customer."),
    // How an order starts.
    ("fnb.placing.created", "Order made from the sales order"),
    // Accepting and rejecting.
    ("fnb.accept.not_placed", "Only a placed order is accepted or rejected."),
    ("fnb.reject.reason_needed", "Give the reason for rejecting the order."),
    // The delivery details.
    ("fnb.details.closed", "The order is out of the door or finished: its details no longer change."),
    ("fnb.details.kind", "Choose delivery or pickup."),
    ("fnb.details.address_needed", "A delivery needs the customer's name and an address."),
    // Dispatch.
    ("fnb.dispatch.not_delivery", "Only an order for delivery is dispatched."),
    (
        "fnb.dispatch.no_rider",
        "Dispatch is not available yet: put the delivery of the order on a run with a rider first.",
    ),
    ("fnb.delivery.customer", "Customer"),
    (
        "fnb.delivery.refused_title",
        "No delivery for order {{ number }}",
    ),
    (
        "fnb.delivery.refused_body",
        "The delivery of order {{ number }} could not be requested. Check the delivery details and request it again.",
    ),
    // Completing.
    ("fnb.completed.title", "Your order {{ number }} is complete"),
    ("fnb.completed.body", "Thank you for your order {{ number }}."),
    // A failed delivery (BL-FNB-01).
    (
        "fnb.failed.reason_needed",
        "A failed delivery needs the reason the rider gave.",
    ),
    (
        "fnb.failed.customer_title",
        "We could not deliver your order {{ number }}",
    ),
    (
        "fnb.failed.customer_body",
        "We could not deliver your order {{ number }}. We will get in touch about it.",
    ),
    ("fnb.failed.not_failed", "The delivery of this order did not fail."),
    ("fnb.failed.party_unknown", "Choose the customer, the rider or the business."),
    ("fnb.failed.party_note_needed", "Give the reason for changing the responsible party."),
    ("fnb.failed.reason.customer_unreachable", "Customer unreachable ({{ delivery }})"),
    ("fnb.failed.reason.wrong_address", "Wrong address ({{ delivery }})"),
    ("fnb.failed.reason.refused", "Refused ({{ delivery }})"),
    ("fnb.failed.reason.refused_late", "Refused, the order was late ({{ delivery }})"),
    ("fnb.failed.reason.refused_wrong_item", "Refused, wrong item ({{ delivery }})"),
    ("fnb.failed.reason.no_cash", "No cash ({{ delivery }})"),
    ("fnb.failed.reason.rider_issue", "Rider issue ({{ delivery }})"),
    ("fnb.failed.reason.run_aborted", "Run aborted ({{ delivery }})"),
    // Closing a failed delivery (BL-FNB-02).
    ("fnb.close.history", "Failed delivery of order {{ number }} closed"),
    (
        "fnb.close.refund_reason",
        "Refund of order {{ number }}: the delivery failed on the outlet's side",
    ),
    (
        "fnb.close.cancel_failed_title",
        "Order {{ number }} could not be cancelled",
    ),
    (
        "fnb.close.cancel_failed_body",
        "The failed delivery of order {{ number }} is closed, but its sales order could not be cancelled and the food was not logged as wastage: {{ problem }}",
    ),
    (
        "fnb.close.refund_failed_title",
        "No refund was asked for order {{ number }}",
    ),
    (
        "fnb.close.refund_failed_body",
        "The failed delivery of order {{ number }} is closed, but the refund could not be asked for: {{ problem }}",
    ),
    ("fnb.refund.not_closed", "A refund is asked for from an order closed after a failed delivery."),
    ("fnb.refund.no_need", "This order was refunded in full or paid in cash, so there is nothing to ask for."),
    ("fnb.refund.window_closed", "refund request window closed"),
    ("fnb.refund.nothing_left", "Everything paid for this order is refunded or asked for."),
    ("fnb.refund.reason_needed", "Give the reason for the refund."),
    ("fnb.refund.amount", "The refund is above zero and not above what is left of what was paid."),
    // Cash on delivery (BL-FNB-03).
    (
        "fnb.cod.off_title",
        "Cash on delivery is off for a customer after order {{ number }}",
    ),
    (
        "fnb.cod.off_body",
        "{{ customer }} had {{ count }} cash orders fail on the customer's side in {{ days }} days (the last: {{ number }}). Cash on delivery is off for this customer. You can turn it back on.",
    ),
    ("fnb.cod.already_on", "Cash on delivery is on for this customer already."),
    // Cancelling an order (BL-FNB-04).
    ("fnb.order_cancelled", "Order cancelled."),
    ("fnb.order_cancel_waiting", "The cancellation waits for a manager's approval."),
    ("fnb.my_order_cancelled", "Your order is cancelled."),
    (
        "fnb.cancel.party_and_reason_needed",
        "A cancellation needs a reason and the party responsible: the business, the aggregator or the customer.",
    ),
    ("fnb.cancel.already_cancelled", "The order is already cancelled."),
    (
        "fnb.cancel.already_accepted",
        "The order is already accepted: the kitchen has it, so it can no longer be cancelled here.",
    ),
    (
        "fnb.cancel.too_late",
        "The order is out of the door or finished: it can no longer be cancelled.",
    ),
    ("fnb.cancel.reason_unknown", "Choose the reason for the cancellation."),
    ("fnb.cancel.note_needed", "Say in the note why the order is cancelled."),
    (
        "fnb.cancel.pending",
        "A change of this order already waits for a manager's approval.",
    ),
    (
        "fnb.cancel.party_unknown",
        "Choose who is responsible: the business, the aggregator or the customer.",
    ),
    (
        "fnb.cancel.not_aggregator",
        "Only an aggregator's order can be cancelled with the aggregator responsible.",
    ),
    (
        "fnb.cancel.outcome_needed",
        "Choose what happens to {{ item }}: the kitchen started it, so return it to stock or scrap it.",
    ),
    ("fnb.cancel.customer_reason", "Cancelled by the customer"),
    ("fnb.cancel.history", "Order {{ number }} cancelled: {{ reason }}"),
    ("fnb.cancel.reason.entered_in_error", "Entered in error"),
    ("fnb.cancel.reason.customer_request", "The customer asked"),
    ("fnb.cancel.reason.item_unavailable", "An item is not available"),
    ("fnb.cancel.reason.kitchen_issue", "The kitchen could not make it"),
    ("fnb.cancel.reason.aggregator_cancelled", "Cancelled by the aggregator"),
    ("fnb.cancel.reason.other", "Other reason"),
    ("fnb.cancel.delivery_reason", "The order was cancelled"),
    ("fnb.cancel.customer_title", "Your order {{ number }} is cancelled"),
    (
        "fnb.cancel.customer_body",
        "Your order {{ number }} is cancelled. Nothing is charged for it.",
    ),
    (
        "fnb.cancel.customer_body_refund",
        "Your order {{ number }} is cancelled. {{ amount }} is refunded to the way you paid.",
    ),
    // Amending an order and cancelling a line (BL-FNB-05).
    ("fnb.order_amended", "Order amended. The kitchen tickets are updated."),
    ("fnb.order_amend_waiting", "The amendment waits for a manager's approval."),
    ("fnb.line_cancelled", "Line cancelled. The kitchen ticket is removed."),
    ("fnb.line_cancel_waiting", "The cancellation of the line waits for a manager's approval."),
    (
        "fnb.amend.aggregator",
        "An aggregator's order is changed on the aggregator's tablet. Record the result here: cancel the line, or cancel the order.",
    ),
    (
        "fnb.amend.agreed",
        "Call the customer first and tick that the customer agreed to the change.",
    ),
    (
        "fnb.amend.started",
        "The kitchen started this line, so it is not amended. Cancel the line and take a new order, or ask the kitchen manager.",
    ),
    (
        "fnb.amend.state",
        "Only a placed or accepted order is amended.",
    ),
    ("fnb.amend.call_time", "Give the time of the call, as hours and minutes, e.g. 18:45."),
    ("fnb.amend.reason", "Say why the order is amended."),
    ("fnb.amend.line", "Choose a line of the order that is still open."),
    ("fnb.amend.quantity", "The quantity is a whole number above zero."),
    ("fnb.amend.price", "Give the price of one unit, in dirhams, above zero."),
    ("fnb.amend.history", "Amended after calling the customer at {{ time }}: {{ reason }}"),
    (
        "fnb.line.reason",
        "A line is cancelled because an item is not available or the customer asked.",
    ),
    ("fnb.line.reason.item_unavailable", "Item not available"),
    ("fnb.line.reason.customer_request", "The customer asked"),
    (
        "fnb.line.bumped",
        "The kitchen already bumped this line, so it is not cancelled. Refund it after the order is completed instead.",
    ),
    ("fnb.line.state", "A line is cancelled while the order is accepted or being prepared."),
    (
        "fnb.line.last",
        "This is the last open line. Cancel the whole order instead.",
    ),
    // The aggregator's rider who does not arrive (BL-FNB-06).
    ("fnb.picked_up_late", "Recorded: the aggregator's rider collected the order late. The order is completed."),
    (
        "fnb.aggregator_cancelled",
        "Recorded: the aggregator cancelled the order. The food is logged as wastage and compensation is expected.",
    ),
    (
        "fnb.pickup.alert_title",
        "Chase the aggregator for order {{ number }}",
    ),
    (
        "fnb.pickup.alert_body",
        "Order {{ number }} has been ready for {{ minutes }} minutes and the aggregator's rider has not collected it. Chase the aggregator on its tablet.",
    ),
    (
        "fnb.pickup.task_title",
        "Record what happened to order {{ number }}",
    ),
    (
        "fnb.pickup.task_body",
        "Order {{ number }} has been ready for {{ minutes }} minutes. Record whether the rider picked it up late or the aggregator cancelled it, on {{ link }}.",
    ),
    ("fnb.pickup.task_closed", "Order {{ number }} left the pass."),
    ("fnb.pickup.late_history", "Order {{ number }} collected late by the aggregator's rider"),
    ("fnb.pickup.cancel_reason", "Cancelled by the aggregator: its rider never arrived"),
    ("fnb.pickup.not_ready", "Only an order that is ready and waiting at the pass is recorded here."),
    ("fnb.pickup.not_aggregator", "Only an aggregator's order is recorded here."),
    // A reversed delivery (BL-FNB-07).
    ("fnb.reversal.reason", "not delivered (delivery reversed)"),
    ("fnb.reversal.customer_title", "Your order {{ number }} is refunded"),
    (
        "fnb.reversal.customer_body_refund",
        "We are sorry your order {{ number }} did not reach you. {{ amount }} is refunded to the way you paid.",
    ),
    (
        "fnb.reversal.customer_body_credit",
        "We are sorry your order {{ number }} did not reach you. You owe nothing for it ({{ amount }} is cleared).",
    ),
    (
        "fnb.reversal.refund_title",
        "Refund of order {{ number }} to pay",
    ),
    (
        "fnb.reversal.refund_body",
        "The delivery of order {{ number }} was reversed. {{ amount }} is refunded to the customer: a manager approves the refund and the cashier records how it is paid.",
    ),
    (
        "fnb.reversal.failed_title",
        "No refund was raised for order {{ number }}",
    ),
    (
        "fnb.reversal.failed_body",
        "The delivery of order {{ number }} was reversed, but the refund could not be raised: {{ problem }}",
    ),
    (
        "fnb.cancel.withdrawn",
        "The kitchen started order {{ number }} while the change waited for a manager: ask again.",
    ),
];

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key, &[])
}

/// The English text for `key` with `params` filled in as `{{ name }}`; the
/// key itself when it has no text.
pub(crate) fn english(key: &str, params: &[(&str, &(dyn Display + Sync))]) -> String {
    let text = TEXTS
        .iter()
        .chain(SCREEN_TEXTS)
        .chain(PAYOUT_TEXTS)
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| *text);
    let mut data = renox::serde_json::Map::new();
    for (name, value) in params {
        data.insert(
            (*name).to_owned(),
            renox::serde_json::Value::String(value.to_string()),
        );
    }
    wl_kernel::notify::fill(text, &renox::serde_json::Value::Object(data))
}

/// The English text of `key`, unfilled: a notification template.
pub(crate) fn template(key: &str) -> &'static str {
    TEXTS
        .iter()
        .chain(SCREEN_TEXTS)
        .chain(PAYOUT_TEXTS)
        .find(|(k, _)| *k == key)
        .map_or("", |(_, text)| *text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_prefixed() {
        let mut keys: Vec<&str> = TEXTS
            .iter()
            .chain(SCREEN_TEXTS)
        .chain(PAYOUT_TEXTS)
            .map(|(key, _)| *key)
            .collect();
        assert!(keys.iter().all(|key| key.starts_with("fnb.")));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a text key is listed twice");
    }

    #[test]
    fn a_text_is_filled_and_an_unknown_key_is_itself() {
        assert_eq!(
            english("fnb.failed.customer_title", &[("number", &"SO-2026-0501")]),
            "We could not deliver your order SO-2026-0501"
        );
        assert_eq!(english("fnb.nothing", &[]), "fnb.nothing");
    }
}
