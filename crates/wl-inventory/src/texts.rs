//! The package's texts, in English. An app changes or translates one by
//! putting the key in its `resources/lang/{locale}.json`.

use renox::prelude::*;

use crate::screen_texts::SCREEN_TEXTS;

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
        "A goods receipt is received against a purchase order or a dispatched return to supplier.",
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
    ("wl_inventory.receipt.line_saved", "Line saved."),
    (
        "wl_inventory.receipt.over_receipt",
        "The accepted quantity is above the order by more than the tolerance; the receipt waits for approval.",
    ),
    ("wl_inventory.receipt.posted", "Goods receipt posted."),
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
    // Refusals of `reverse` on a goods receipt.
    (
        "wl_inventory.reverse.not_posted",
        "Only a posted goods receipt can be reversed.",
    ),
    (
        "wl_inventory.reverse.return_left",
        "Return {{ number }} from this receipt has left with the supplier; it cannot be reversed. Correct it with a credit from the supplier.",
    ),
    (
        "wl_inventory.reverse.billed",
        "The purchase order is closed: its goods were billed, so the receipt cannot be reversed.",
    ),
    (
        "wl_inventory.reverse.missing",
        "Line {{ position }}: {{ item }} has {{ available }} left at the location of the {{ accepted }} accepted, {{ missing }} missing. Correct it with a stock adjustment or a return to supplier.",
    ),
    (
        "wl_inventory.reverse.held",
        "Line {{ position }}: lot {{ lot }} of {{ item }} is not free ({{ status }}). Correct it with a stock adjustment or a return to supplier.",
    ),
    (
        "wl_inventory.reverse.replacement",
        "This receipt is a replacement counted on a return to supplier; it cannot be reversed.",
    ),
    ("wl_inventory.reverse.reason", "Goods receipt reversed"),
    ("wl_inventory.reverse.return_cancelled", "receipt reversed"),
    ("wl_inventory.receipt.reversed", "Goods receipt reversed."),
    // Refusals of `dispatch` on a Return to supplier.
    (
        "wl_inventory.return.not_draft",
        "Only a draft return to supplier can be dispatched.",
    ),
    (
        "wl_inventory.return.no_lines",
        "A return to supplier needs a supplier and at least one line.",
    ),
    (
        "wl_inventory.return.bad_line",
        "Line {{ position }}: give a quantity above 0 and a reason.",
    ),
    (
        "wl_inventory.return.proof_needed",
        "Give the collection proof: the driver's signature, a courier number or the goods receipt.",
    ),
    (
        "wl_inventory.return.no_location",
        "Line {{ position }}: the return names no location to take the stock from.",
    ),
    (
        "wl_inventory.return.not_in_stock",
        "Line {{ position }}: {{ item }} has {{ available }} at the location, {{ quantity }} are to go back.",
    ),
    (
        "wl_inventory.return.order_closed",
        "Line {{ position }}: the purchase order is closed, so the goods can only be credited, not replaced on it.",
    ),
    (
        "wl_inventory.return.not_at_door",
        "Only goods refused at the door are dispatched by the system.",
    ),
    (
        "wl_inventory.return.dispatched",
        "Return to supplier dispatched.",
    ),
    (
        "wl_inventory.return.waiting",
        "The return is worth more than the limit and waits for a second person to approve it.",
    ),
    (
        "wl_inventory.return.bad_proof",
        "The collection proof is a driver signature, a courier number or a goods receipt.",
    ),
    // A free replacement received against a Return to supplier.
    (
        "wl_inventory.receipt.unknown_return",
        "The return to supplier of this receipt does not exist.",
    ),
    (
        "wl_inventory.receipt.return_not_dispatched",
        "A replacement is received against a dispatched return to supplier.",
    ),
    (
        "wl_inventory.receipt.line_not_on_return",
        "Line {{ position }} names no line of the return to supplier.",
    ),
    (
        "wl_inventory.receipt.return_line_not_replace",
        "Line {{ position }}: the return line is to be credited, not replaced.",
    ),
    (
        "wl_inventory.receipt.return_line_on_open_order",
        "Line {{ position }}: the replacement is received on its purchase order, which is still open.",
    ),
    (
        "wl_inventory.receipt.above_returned",
        "Line {{ position }} of the return: {{ remaining }} are still to be replaced, {{ received }} were received.",
    ),
    // `cancel` on a Return to supplier.
    (
        "wl_inventory.return.cancel_draft_only",
        "Only a draft return to supplier can be cancelled.",
    ),
    (
        "wl_inventory.return.kept_goods_needed",
        "Say what becomes of the rejected goods kept for collection: receive them on a new goods receipt, or discard them.",
    ),
    (
        "wl_inventory.return.cancelled",
        "Return to supplier cancelled.",
    ),
    (
        "wl_inventory.return.bad_kept_goods",
        "The kept goods are received again or discarded.",
    ),
    ("wl_inventory.return.reason_needed", "Give a reason."),
    // `credit` on a Return to supplier.
    (
        "wl_inventory.return.credit_dispatched_only",
        "Only a dispatched return to supplier can be credited.",
    ),
    (
        "wl_inventory.return.credited",
        "Return to supplier credited.",
    ),
    (
        "wl_inventory.return.credit_waiting",
        "The refused credit is worth more than the limit and waits for a second person to approve it.",
    ),
    (
        "wl_inventory.return.bad_basis",
        "The credit is settled as refund received or supplier refused.",
    ),
    (
        "wl_inventory.return.debit_note_cancelled_title",
        "Debit note of return {{ number }} cancelled",
    ),
    (
        "wl_inventory.return.debit_note_cancelled",
        "The supplier debit note of return {{ number }} was cancelled ({{ reason }}). The return stays with the supplier: chase the credit or a replacement.",
    ),
    // `STK.RUL.08`: chasing a return that stays dispatched.
    (
        "wl_inventory.chase.subject",
        "Return {{ number }} still not credited after {{ days }} days",
    ),
    (
        "wl_inventory.chase.body",
        "Return {{ number }} to the supplier was dispatched on {{ dispatched }} and is neither credited nor replaced after {{ days }} days. Procurement chases the supplier; finance, expect the credit.",
    ),
    ("wl_inventory.chase.resolved", "The return was credited."),
    (
        "wl_inventory.chase.task_title",
        "Chase the supplier for return {{ number }}",
    ),
    (
        "wl_inventory.chase.task",
        "Return {{ number }} has been dispatched for {{ days }} days without a credit or a replacement. Ask the supplier for the debit note or the replacement.",
    ),
    // Revising an issued purchase order.
    (
        "wl_inventory.revise.not_open",
        "Only an issued or partially received purchase order can be revised ({{ status }}).",
    ),
    (
        "wl_inventory.revise.reason_needed",
        "Give a reason for the revision.",
    ),
    (
        "wl_inventory.revise.currency",
        "The currency of a purchase order cannot change: cancel it and make a new order.",
    ),
    (
        "wl_inventory.revise.unknown_line",
        "Line {{ line }} is not a line of this purchase order.",
    ),
    (
        "wl_inventory.revise.negative",
        "Line {{ position }}: the quantity and the price cannot be negative.",
    ),
    (
        "wl_inventory.revise.below_accepted",
        "Line {{ position }}: {{ accepted }} are already accepted, so the quantity cannot go below that.",
    ),
    (
        "wl_inventory.revise.date_asker",
        "Say who asked for the later expected date: us or the supplier.",
    ),
    ("wl_inventory.revise.nothing", "Nothing was changed."),
    (
        "wl_inventory.revise.pending",
        "A revision of this order already waits for approval.",
    ),
    (
        "wl_inventory.revise.moved",
        "The order was revised again while this revision waited; ask again.",
    ),
    (
        "wl_inventory.revise.bad_input",
        "The revision is not complete.",
    ),
    ("wl_inventory.revise.revised", "Purchase order revised."),
    (
        "wl_inventory.revise.waiting",
        "The new total is above a limit and waits for approval; the current revision stays in force.",
    ),
    (
        "wl_inventory.revise.bad_number",
        "Enter the quantity and the price of a line as numbers.",
    ),
    (
        "wl_inventory.revise.bad_date",
        "Enter the expected date like 2027-01-31.",
    ),
    (
        "wl_inventory.revise.subject",
        "Purchase order {{ number }}, revision {{ revision }}",
    ),
    (
        "wl_inventory.revise.body",
        "Purchase order {{ number }} was revised. Revision {{ revision }} is in force: the total is now {{ total }} {{ currency }}. Reason: {{ reason }}. The document: {{ link }}",
    ),
    // Landed costs and their adjustments.
    (
        "wl_inventory.landed.draft_needs",
        "A landed cost needs at least one receipt and one cost line.",
    ),
    (
        "wl_inventory.landed.receipt_not_posted",
        "Goods receipt {{ receipt }} is not posted yet.",
    ),
    (
        "wl_inventory.landed.no_receipts",
        "The receipts of the landed cost have no accepted goods to spread the cost over.",
    ),
    (
        "wl_inventory.landed.no_costs",
        "The landed cost has no cost to spread.",
    ),
    (
        "wl_inventory.landed.weight_needed",
        "Receipt line {{ line }} needs a weight or volume above zero.",
    ),
    (
        "wl_inventory.landed.bad_input",
        "The landed cost action is not complete.",
    ),
    (
        "wl_inventory.landed.not_allocated",
        "Only an allocated landed cost can be adjusted.",
    ),
    (
        "wl_inventory.landed.not_draft",
        "The adjustment is not a draft any more.",
    ),
    (
        "wl_inventory.landed.no_lines",
        "An adjustment needs at least one cost line.",
    ),
    (
        "wl_inventory.landed.bill_reference",
        "Give the supplier bill's reference.",
    ),
    (
        "wl_inventory.landed.zero_total",
        "The cost lines add up to nothing.",
    ),
    (
        "wl_inventory.landed.no_adjustment",
        "There is no such adjustment on this landed cost.",
    ),
    (
        "wl_inventory.landed.period_locked",
        "The posting date is in a period that was already filed; pick a date in an open period.",
    ),
    (
        "wl_inventory.landed.pending",
        "This adjustment already waits for approval.",
    ),
    (
        "wl_inventory.landed.adjusted",
        "Landed cost adjustment posted.",
    ),
    (
        "wl_inventory.landed.waiting",
        "The adjustment is above the value and waits for approval; nothing is posted until then.",
    ),
    (
        "wl_inventory.landed.drafted_subject",
        "Landed cost {{ number }}: a bill of {{ amount }} AED waits to be posted",
    ),
    (
        "wl_inventory.landed.drafted_body",
        "Bill {{ bill }} was approved for the allocated landed cost {{ number }}. A draft adjustment of {{ amount }} AED was made for you to check and post: {{ link }}",
    ),
    ("wl_inventory.landed.allocated", "Landed cost allocated."),
    (
        "wl_inventory.landed.drafted",
        "Adjustment saved as a draft.",
    ),
    (
        "wl_inventory.landed.bad_kind",
        "A cost line is freight, duty, clearing or insurance.",
    ),
    (
        "wl_inventory.landed.bad_number",
        "Enter the amounts as numbers.",
    ),
    (
        "wl_inventory.landed.bad_date",
        "Enter the posting date like 2027-01-31.",
    ),
    // Issuing a purchase order.
    (
        "wl_inventory.issue.not_draft",
        "Only a draft purchase order can be issued ({{ status }}).",
    ),
    (
        "wl_inventory.issue.no_supplier",
        "Choose the supplier before the purchase order is issued.",
    ),
    (
        "wl_inventory.issue.no_location",
        "Choose the delivery location before the purchase order is issued.",
    ),
    (
        "wl_inventory.issue.no_lines",
        "A purchase order needs at least one line.",
    ),
    (
        "wl_inventory.issue.no_price",
        "Line {{ position }} needs a quantity and a price.",
    ),
    ("wl_inventory.issue.issued", "Purchase order issued."),
    (
        "wl_inventory.issue.exception_asked",
        "The exception was asked for; the purchase order stays a draft until it is approved.",
    ),
    (
        "wl_inventory.issue.approval_asked",
        "The purchase order was sent for approval; it stays a draft until it is approved.",
    ),
    // The suppliers of an item and the reorder suggestions.
    (
        "wl_inventory.supplier.twice",
        "The item already lists this supplier.",
    ),
    (
        "wl_inventory.supplier.needs_request",
        "The item has, or had, a preferred supplier: changing it needs a request that a second person approves.",
    ),
    (
        "wl_inventory.supplier.inactive",
        "This supplier is not in use for the item.",
    ),
    (
        "wl_inventory.supplier.negative",
        "The lead time and the minimum order cannot be negative.",
    ),
    ("wl_inventory.supplier.added", "Supplier added to the item."),
    (
        "wl_inventory.supplier.deactivated",
        "Supplier taken out of use for the item.",
    ),
    (
        "wl_inventory.supplier.requested",
        "The supplier is the preferred one: a request was opened for a second person to approve.",
    ),
    (
        "wl_inventory.supplier.bad_input",
        "Enter the supplier, the role, the lead time and the minimum order as numbers.",
    ),
    (
        "wl_inventory.reorder.preferred_on_hold",
        "preferred supplier on hold",
    ),
    (
        "wl_inventory.reorder.preferred_inactive",
        "preferred supplier inactive",
    ),
    (
        "wl_inventory.reorder.switched",
        "supplier switched by the purchaser",
    ),
    (
        "wl_inventory.reorder.reason_needed",
        "Give the reason for switching the supplier.",
    ),
    (
        "wl_inventory.reorder.not_open",
        "Only an open suggestion can be switched.",
    ),
    (
        "wl_inventory.reorder.not_listed",
        "The supplier is not listed and in use for the item.",
    ),
    (
        "wl_inventory.reorder.same",
        "The suggestion already goes to this supplier.",
    ),
    ("wl_inventory.reorder.on_hold", "This supplier is on hold."),
    (
        "wl_inventory.reorder.switched_done",
        "Suggestion switched to the other supplier.",
    ),
    (
        "wl_inventory.reorder.subject",
        "{{ count }} reorder suggestion(s) for you",
    ),
    (
        "wl_inventory.reorder.body",
        "Today's run found {{ count }} item(s) at or below their reorder point. Open the suggestions to choose the supplier and order.",
    ),
    // The supplier score.
    ("wl_inventory.score.note_needed", "Write the review note."),
    ("wl_inventory.score.noted", "Review note added."),
    // Changing the preferred supplier of an item (`STK.ACL.07`).
    (
        "wl_inventory.preferred.same_person",
        "A second person decides: the requester cannot approve or reject their own request.",
    ),
    (
        "wl_inventory.preferred.only_requester",
        "Only the requester can withdraw the request.",
    ),
    (
        "wl_inventory.preferred.not_pending",
        "This request has been decided already.",
    ),
    (
        "wl_inventory.preferred.pending_exists",
        "A request to change the preferred supplier of this item is pending: decide or withdraw it first.",
    ),
    (
        "wl_inventory.preferred.add_directly",
        "The item never had a preferred supplier: add it as the preferred supplier, no request is needed.",
    ),
    (
        "wl_inventory.preferred.already",
        "This supplier is the preferred supplier of the item already.",
    ),
    (
        "wl_inventory.preferred.stale",
        "The suppliers of the item changed since the request was made; ask again.",
    ),
    (
        "wl_inventory.preferred.new_on_hold",
        "The new supplier is on hold now: the request is returned and nothing changed.",
    ),
    (
        "wl_inventory.preferred.cancel_row_off",
        "The supplier was taken out of use for the item.",
    ),
    (
        "wl_inventory.preferred.cancel_archived",
        "The item was archived.",
    ),
    (
        "wl_inventory.preferred.reason_needed",
        "Give the reason for rejecting the request.",
    ),
    (
        "wl_inventory.preferred.requested",
        "Request sent: a second person approves the change.",
    ),
    (
        "wl_inventory.preferred.applied",
        "The preferred supplier is changed.",
    ),
    ("wl_inventory.preferred.approved", "Request approved."),
    ("wl_inventory.preferred.rejected", "Request rejected."),
    ("wl_inventory.preferred.withdrawn", "Request withdrawn."),
    (
        "wl_inventory.preferred.subject",
        "Preferred supplier of {{ item }}: {{ outcome }}",
    ),
    (
        "wl_inventory.preferred.body",
        "Your request to change the preferred supplier of {{ item }} was {{ outcome }}. {{ note }}",
    ),
    (
        "wl_inventory.preferred.task_title",
        "Decide the preferred supplier change for {{ item }}",
    ),
    (
        "wl_inventory.preferred.task_body",
        "Open request {{ request }} to see both suppliers' scores, lead times and last prices, then approve or reject it.",
    ),
    (
        "wl_inventory.preferred.task_done",
        "The request was decided.",
    ),
    // The review of a supplier under the threshold.
    (
        "wl_inventory.review.title",
        "Review supplier {{ supplier }}: score under the threshold",
    ),
    (
        "wl_inventory.review.breakdown",
        "Score {{ score }} for {{ from }} to {{ to }} on {{ receipts }} receipts. On time {{ on_time }}, rejected value {{ rejected }}, incidents {{ incidents }}, price variance {{ price }}.",
    ),
    (
        "wl_inventory.review.item",
        "Preferred for {{ item }}; alternatives: {{ alternatives }}.",
    ),
    (
        "wl_inventory.review.subject",
        "Supplier {{ supplier }} scored {{ score }}",
    ),
    (
        "wl_inventory.review.body",
        "The monthly score of supplier {{ supplier }} is {{ score }}, under the threshold. Procurement has a review task to decide on.",
    ),
    ("wl_inventory.review.not_open", "This review is closed."),
    (
        "wl_inventory.review.items_needed",
        "Choose the items whose preferred supplier should change.",
    ),
    (
        "wl_inventory.review.not_preferred",
        "The supplier is not the preferred supplier of item {{ item }}.",
    ),
    (
        "wl_inventory.review.bad_decision",
        "Decide to keep, watch, change the preferred supplier or request a block.",
    ),
    ("wl_inventory.review.closed", "Review closed."),
    ("wl_inventory.review.resolved", "Reviewed and decided."),
    (
        "wl_inventory.review.block_title",
        "Decide whether to block supplier {{ supplier }}",
    ),
    (
        "wl_inventory.review.block_body",
        "Procurement asks for a decision on blocking the supplier after review {{ review }}. Nothing is blocked until a holder of parties.block does it.",
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
        .chain(SCREEN_TEXTS)
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
        for (key, _) in TEXTS.iter().chain(SCREEN_TEXTS) {
            assert!(key.starts_with("wl_inventory."), "{key}");
            assert!(seen.insert(*key), "{key} twice");
        }
    }
}
