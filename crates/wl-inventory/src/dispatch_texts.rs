//! The texts of the dispatch (the refusals of its transitions and its
//! screens), in English. An app changes or translates one by putting the key
//! in its `resources/lang/{locale}.json`. The keys under
//! `wl_inventory.dispatch.ui.` are the screen's: a template reads them as
//! `tx['name']`.

/// Every key of the dispatch, with its English text.
pub(crate) const DISPATCH_TEXTS: &[(&str, &str)] = &[
    // Refusals of making, picking and cancelling.
    (
        "wl_inventory.dispatch.no_lines",
        "A dispatch needs at least one line.",
    ),
    (
        "wl_inventory.dispatch.bad_line",
        "Line {{ position }}: the quantity must be above zero.",
    ),
    (
        "wl_inventory.dispatch.lot_needed",
        "Line {{ position }}: {{ item }} is lot-tracked; name the lot.",
    ),
    (
        "wl_inventory.dispatch.cancel_task_title",
        "{{ dispatch }}: a cancelled order line, goods already dispatched",
    ),
    (
        "wl_inventory.dispatch.cancel_task_body",
        "Sale line {{ line }} of the order was cancelled after {{ dispatch }} left (now {{ status }}). The dispatch is not changed: the goods follow the delivery, returned or refused. Follow it up with the delivery.",
    ),
    (
        "wl_inventory.dispatch.not_draft",
        "Only a draft dispatch can be picked.",
    ),
    (
        "wl_inventory.dispatch.not_picked",
        "Only a picked dispatch can be dispatched or re-picked.",
    ),
    (
        "wl_inventory.dispatch.not_dispatched",
        "Only a dispatched dispatch can be confirmed delivered.",
    ),
    (
        "wl_inventory.dispatch.lot_on_hold",
        "Line {{ position }}: lot {{ lot }} is on hold.",
    ),
    (
        "wl_inventory.dispatch.lot_not_released",
        "Line {{ position }}: lot {{ lot }} is not released.",
    ),
    (
        "wl_inventory.dispatch.lot_expired",
        "Line {{ position }}: lot {{ lot }} is past its expiry date.",
    ),
    (
        "wl_inventory.dispatch.lot_other_item",
        "Lot {{ lot }} is not a lot of {{ item }}.",
    ),
    (
        "wl_inventory.dispatch.held",
        "Line {{ position }}: {{ held }} of {{ item }} is on hold.",
    ),
    (
        "wl_inventory.dispatch.not_available",
        "Line {{ position }}: only {{ available }} of {{ item }} is available for {{ quantity }}.",
    ),
    (
        "wl_inventory.dispatch.not_in_stock",
        "Line {{ position }}: only {{ available }} of {{ item }} is in stock for {{ quantity }}.",
    ),
    (
        "wl_inventory.dispatch.repick",
        "Re-pick the line from another released lot.",
    ),
    // Refusals of `dispatch`.
    (
        "wl_inventory.dispatch.handover_needed",
        "Choose how the goods are handed over: our own delivery, a courier or collection by the customer.",
    ),
    (
        "wl_inventory.dispatch.tracking_needed",
        "Give the courier's tracking number.",
    ),
    (
        "wl_inventory.dispatch.signature_needed",
        "The collector must sign for the goods: give the signature reference.",
    ),
    (
        "wl_inventory.dispatch.recipient_needed",
        "An own delivery needs the recipient's name.",
    ),
    (
        "wl_inventory.dispatch.credit_limit",
        "The customer's credit re-check failed: the exposure is above the limit. The order is on credit hold and the dispatch stays picked, its stock reserved.",
    ),
    (
        "wl_inventory.dispatch.credit_overdue",
        "The customer's credit re-check failed: an invoice is overdue. The order is on credit hold and the dispatch stays picked, its stock reserved.",
    ),
    (
        "wl_inventory.dispatch.credit_account_on_hold",
        "The customer's credit re-check failed: the account is on hold. The order is on credit hold and the dispatch stays picked, its stock reserved.",
    ),
    (
        "wl_inventory.dispatch.delivery_cancelled",
        "The dispatch could not leave.",
    ),
    // Refusals of `confirm_delivery` and of the refused goods coming back.
    (
        "wl_inventory.dispatch.proof_from_delivery",
        "An own delivery is confirmed by the driver's proof of delivery, not by hand.",
    ),
    (
        "wl_inventory.dispatch.line_missing",
        "Line {{ position }}: enter the delivered and the refused quantity.",
    ),
    (
        "wl_inventory.dispatch.bad_quantity",
        "Line {{ position }}: quantities cannot be negative.",
    ),
    (
        "wl_inventory.dispatch.too_much",
        "Line {{ position }}: delivered plus refused is more than the {{ quantity }} that was sent.",
    ),
    (
        "wl_inventory.dispatch.reason_needed",
        "Line {{ position }}: give the reason the goods were refused.",
    ),
    (
        "wl_inventory.dispatch.nothing_entered",
        "Enter the quantities from the proof of delivery.",
    ),
    (
        "wl_inventory.dispatch.not_left",
        "Refused goods come back only for a dispatch that has left.",
    ),
    (
        "wl_inventory.dispatch.more_than_in_return",
        "Line {{ position }}: only {{ quantity }} is still in return.",
    ),
    (
        "wl_inventory.dispatch.missing_title",
        "Goods missing on return: dispatch {{ number }}",
    ),
    (
        "wl_inventory.dispatch.missing_body",
        "{{ quantity }} of the refused goods of dispatch {{ number }} did not come back. The value is written off as missing on return. Find out what happened to them.",
    ),
    (
        "wl_inventory.dispatch.missing_approval",
        "The loss is above the limit: a second person must approve it (STK.ACL.02).",
    ),
    (
        "wl_inventory.dispatch.complaint_title",
        "Returned delivery: dispatch {{ number }}",
    ),
    (
        "wl_inventory.dispatch.complaint_body",
        "Dispatch {{ number }} came back. Reason: {{ reason }}. The goods are in quarantine and wait for an inspection; sales decides whether to deliver again or cancel.",
    ),
    // Words.
    ("wl_inventory.dispatch.ui.status.draft", "Draft"),
    ("wl_inventory.dispatch.ui.status.picked", "Picked"),
    ("wl_inventory.dispatch.ui.status.dispatched", "Dispatched"),
    ("wl_inventory.dispatch.ui.status.delivered", "Delivered"),
    ("wl_inventory.dispatch.ui.status.returned", "Returned"),
    ("wl_inventory.dispatch.ui.status.discarded", "Discarded"),
    ("wl_inventory.dispatch.ui.status.lost", "Lost"),
    ("wl_inventory.dispatch.ui.status.cancelled", "Cancelled"),
    (
        "wl_inventory.dispatch.ui.handover.own_delivery",
        "Our own delivery",
    ),
    ("wl_inventory.dispatch.ui.handover.courier", "Courier"),
    (
        "wl_inventory.dispatch.ui.handover.collection",
        "Collected by the customer",
    ),
    ("wl_inventory.dispatch.ui.reason.damaged", "Damaged"),
    ("wl_inventory.dispatch.ui.reason.wrong_item", "Wrong item"),
    ("wl_inventory.dispatch.ui.reason.not_ordered", "Not ordered"),
    ("wl_inventory.dispatch.ui.reason.short_dated", "Short-dated"),
    ("wl_inventory.dispatch.ui.reason.temperature", "Temperature"),
    ("wl_inventory.dispatch.ui.reason.other", "Other"),
    // The list page.
    ("wl_inventory.dispatch.ui.title", "Dispatches"),
    (
        "wl_inventory.dispatch.ui.subtitle",
        "Goods picked, packed and sent to a customer, and what happened to them on the way.",
    ),
    ("wl_inventory.dispatch.ui.stock", "Stock"),
    ("wl_inventory.dispatch.ui.number", "Dispatch"),
    ("wl_inventory.dispatch.ui.recipient", "Recipient"),
    ("wl_inventory.dispatch.ui.address", "Address"),
    ("wl_inventory.dispatch.ui.source", "For"),
    ("wl_inventory.dispatch.ui.store", "Leaves from"),
    ("wl_inventory.dispatch.ui.status", "Status"),
    ("wl_inventory.dispatch.ui.lines", "Lines"),
    ("wl_inventory.dispatch.ui.when", "Changed"),
    ("wl_inventory.dispatch.ui.open", "Open"),
    ("wl_inventory.dispatch.ui.empty", "No dispatch yet"),
    ("wl_inventory.dispatch.ui.none", "None"),
    // The record page.
    ("wl_inventory.dispatch.ui.details", "Dispatch"),
    ("wl_inventory.dispatch.ui.handover", "Hand-over"),
    ("wl_inventory.dispatch.ui.tracking", "Tracking number"),
    (
        "wl_inventory.dispatch.ui.signature",
        "Collector's signature",
    ),
    ("wl_inventory.dispatch.ui.delivery", "Delivery"),
    ("wl_inventory.dispatch.ui.delivery_note", "Delivery note"),
    ("wl_inventory.dispatch.ui.picked_by", "Picked"),
    ("wl_inventory.dispatch.ui.dispatched_at", "Dispatched"),
    ("wl_inventory.dispatch.ui.delivered_at", "Delivered"),
    ("wl_inventory.dispatch.ui.loss", "Loss pending the claim"),
    ("wl_inventory.dispatch.ui.item", "Item"),
    ("wl_inventory.dispatch.ui.lot", "Lot"),
    ("wl_inventory.dispatch.ui.quantity", "Sent"),
    ("wl_inventory.dispatch.ui.delivered", "Delivered"),
    ("wl_inventory.dispatch.ui.refused", "Refused"),
    ("wl_inventory.dispatch.ui.reason", "Reason"),
    ("wl_inventory.dispatch.ui.in_return", "In return"),
    ("wl_inventory.dispatch.ui.received_back", "Received back"),
    ("wl_inventory.dispatch.ui.missing", "Missing"),
    ("wl_inventory.dispatch.ui.no_lot", "No lot"),
    (
        "wl_inventory.dispatch.ui.lot_held",
        "Line {{ position }}: lot {{ lot }} is on hold. Re-pick the line from another released lot before the dispatch can leave.",
    ),
    (
        "wl_inventory.dispatch.ui.lot_unusable",
        "Line {{ position }}: lot {{ lot }} cannot be dispatched any more. Re-pick the line from another released lot.",
    ),
    (
        "wl_inventory.dispatch.ui.lost",
        "The delivery was marked lost. {{ value }} is held as a loss while the claim runs; if the goods are found it is reversed.",
    ),
    (
        "wl_inventory.dispatch.ui.in_return_note",
        "{{ quantity }} refused goods are still with the driver. They go to quarantine when they are brought back.",
    ),
    // The actions.
    ("wl_inventory.dispatch.ui.pick", "Pick"),
    (
        "wl_inventory.dispatch.ui.pick_description",
        "Reserves the stock. Only released lots can be picked and nothing on hold.",
    ),
    ("wl_inventory.dispatch.ui.dispatch", "Dispatch"),
    (
        "wl_inventory.dispatch.ui.dispatch_description",
        "The goods leave stock with the delivery note. The customer's credit is checked again first.",
    ),
    ("wl_inventory.dispatch.ui.cancel", "Cancel"),
    (
        "wl_inventory.dispatch.ui.cancel_description",
        "The goods are not sent and the reservation is released.",
    ),
    ("wl_inventory.dispatch.ui.cancel_reason", "Why"),
    ("wl_inventory.dispatch.ui.return", "Return"),
    (
        "wl_inventory.dispatch.ui.return_description",
        "The goods came back to the store. They go into quarantine and a complaint is opened.",
    ),
    ("wl_inventory.dispatch.ui.return_reason", "Why it came back"),
    (
        "wl_inventory.dispatch.ui.returned_done",
        "The dispatch is returned.",
    ),
    (
        "wl_inventory.dispatch.ui.discarded_note",
        "Destroyed after a failed delivery: {{ value }} written off as wastage. {{ reason }}",
    ),
    (
        "wl_inventory.dispatch.ui.returned_note",
        "Came back: {{ reason }}. The goods are in quarantine.",
    ),
    ("wl_inventory.dispatch.ui.confirm", "Confirm delivery"),
    (
        "wl_inventory.dispatch.ui.confirm_description",
        "Enter what the courier's proof says for every line. Refused goods stay in return until they are brought back.",
    ),
    (
        "wl_inventory.dispatch.ui.receive_back",
        "Refused goods back",
    ),
    (
        "wl_inventory.dispatch.ui.receive_back_description",
        "Enter what the driver brought back and what is missing. What came back goes to quarantine, never to free stock.",
    ),
    ("wl_inventory.dispatch.ui.repick", "Re-pick"),
    (
        "wl_inventory.dispatch.ui.repick_description",
        "Pick this line again from another released lot. Leave the lot empty to take the first to expire.",
    ),
    ("wl_inventory.dispatch.ui.new_lot", "Lot"),
    ("wl_inventory.dispatch.ui.suggest", "First to expire"),
    ("wl_inventory.dispatch.ui.tracking_field", "Tracking number"),
    (
        "wl_inventory.dispatch.ui.signature_field",
        "Signature reference",
    ),
    ("wl_inventory.dispatch.ui.received_field", "Received back"),
    ("wl_inventory.dispatch.ui.missing_field", "Missing"),
    ("wl_inventory.dispatch.ui.delivered_field", "Delivered"),
    ("wl_inventory.dispatch.ui.refused_field", "Refused"),
    (
        "wl_inventory.dispatch.ui.blocked_own_delivery",
        "An own delivery is confirmed by the driver's proof of delivery.",
    ),
    (
        "wl_inventory.dispatch.ui.blocked_nothing_in_return",
        "No refused goods are waiting to come back.",
    ),
    // Toasts and problems.
    ("wl_inventory.dispatch.ui.picked", "The dispatch is picked."),
    (
        "wl_inventory.dispatch.ui.dispatched",
        "The dispatch has left.",
    ),
    (
        "wl_inventory.dispatch.ui.cancelled",
        "The dispatch is cancelled.",
    ),
    (
        "wl_inventory.dispatch.ui.confirmed",
        "The delivery is confirmed.",
    ),
    (
        "wl_inventory.dispatch.ui.received",
        "The refused goods are recorded.",
    ),
    (
        "wl_inventory.dispatch.ui.repicked",
        "The line is picked again.",
    ),
    (
        "wl_inventory.dispatch.ui.problem_quantity",
        "Line {{ position }}: enter a quantity like 5 or 0.5.",
    ),
    (
        "wl_inventory.dispatch.ui.problem_handover",
        "Choose the hand-over.",
    ),
];
