//! The vertical's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use std::fmt::Display;

use renox::prelude::*;

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
        "Dispatch is not available yet: put the order's delivery on a run with a rider first.",
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
