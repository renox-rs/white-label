//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

/// Every key the package uses, with its English text.
const TEXTS: &[(&str, &str)] = &[
    // Refusals of `post` on a goods receipt.
    (
        "wl_inventory.receipt.not_draft",
        "Only a draft goods receipt can be posted.",
    ),
    (
        "wl_inventory.receipt.no_lines",
        "A goods receipt needs at least one line.",
    ),
    (
        "wl_inventory.receipt.no_order",
        "A goods receipt is received against a purchase order.",
    ),
    (
        "wl_inventory.receipt.order_not_open",
        "The purchase order is not open for receiving ({{ status }}).",
    ),
    (
        "wl_inventory.receipt.line_not_on_order",
        "Line {{ position }} names no line of the purchase order.",
    ),
    (
        "wl_inventory.receipt.item_differs",
        "Line {{ position }} is not the item of the purchase order line.",
    ),
    (
        "wl_inventory.receipt.lot_needed",
        "Line {{ position }}: {{ item }} is lot-tracked; give the lot number and the expiry date.",
    ),
    (
        "wl_inventory.receipt.not_balanced",
        "Line {{ position }}: accepted plus rejected must equal what was received.",
    ),
    (
        "wl_inventory.receipt.negative",
        "Line {{ position }}: quantities cannot be negative.",
    ),
    (
        "wl_inventory.receipt.nothing_received",
        "Nothing was received on this receipt.",
    ),
    (
        "wl_inventory.receipt.reason_needed",
        "Line {{ position }}: give a reason for the rejected quantity.",
    ),
    (
        "wl_inventory.receipt.reason_unknown",
        "Line {{ position }}: `{{ reason }}` is not a rejection reason.",
    ),
    (
        "wl_inventory.receipt.handling_needed",
        "Line {{ position }}: say whether the rejected goods were refused at the door or kept for collection.",
    ),
    (
        "wl_inventory.receipt.evidence_needed",
        "Line {{ position }}: a rejection for `{{ reason }}` needs a reading or a photo.",
    ),
    (
        "wl_inventory.receipt.line_saved",
        "Line saved.",
    ),
    (
        "wl_inventory.receipt.over_receipt",
        "The accepted quantity is above the order by more than the tolerance; the receipt waits for approval.",
    ),
    (
        "wl_inventory.receipt.posted",
        "Goods receipt posted.",
    ),
    (
        "wl_inventory.receipt.waiting",
        "The receipt is above the tolerance of the order and waits for approval.",
    ),
    (
        "wl_inventory.receipt.unknown_order",
        "There is no such purchase order.",
    ),
    (
        "wl_inventory.receipt.unknown_location",
        "There is no such location.",
    ),
    (
        "wl_inventory.receipt.unknown_item",
        "Line {{ position }}: there is no such item.",
    ),
    // Refusals of the purchase order's `receive`.
    (
        "wl_inventory.order.over_tolerance",
        "Line {{ position }}: the accepted quantity is above the ordered quantity by more than the tolerance.",
    ),
    (
        "wl_inventory.order.bad_input",
        "The receipt's quantities are missing.",
    ),
    // Names of the rejected goods' returns.
    (
        "wl_inventory.return.collection_proof",
        "Goods receipt {{ receipt }}",
    ),
];

/// The English text for `key`; the key itself when there is none.
pub(crate) fn english(key: &str) -> &str {
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, text)| text)
}

/// The text of `key` filled with `values`.
pub(crate) fn said(key: &str, values: &[(&str, &str)]) -> String {
    let mut data = renox::serde_json::Map::new();
    for (name, value) in values {
        data.insert((*name).to_owned(), renox::serde_json::json!(value));
    }
    wl_kernel::notify::fill(english(key), &renox::serde_json::Value::Object(data))
}

/// The text for `key`: the app's translation when it has one, else the English one.
pub(crate) fn t(lang: &Lang, key: &str) -> String {
    let found = lang.t(key, &[]);
    if found != key {
        return found;
    }
    english(key).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_filled_and_an_unknown_key_is_itself() {
        let text = said("wl_inventory.receipt.not_balanced", &[("position", "2")]);
        assert!(text.starts_with("Line 2:"), "{text}");
        assert_eq!(english("wl_inventory.nothing"), "wl_inventory.nothing");
    }

    #[test]
    fn every_key_is_in_the_package_namespace_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (key, _) in TEXTS {
            assert!(key.starts_with("wl_inventory."), "{key}");
            assert!(seen.insert(*key), "{key} twice");
        }
    }
}
