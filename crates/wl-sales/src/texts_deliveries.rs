//! The texts of the order's deliveries (BL-SAL-13), in English: the tasks of
//! the sales rep and the accountant, the notice of a returned delivery and the
//! words of the fulfilment progress. [`crate::texts`] reads this list as well.

/// Every key of the deliveries, with its English text.
pub(crate) const DELIVERY_TEXTS: &[(&str, &str)] = &[
    // Why a delivery could not be fulfilled on the order.
    (
        "wl_sales.deliveries.status.draft",
        "the order is still a draft",
    ),
    (
        "wl_sales.deliveries.status.awaiting_payment",
        "the order is waiting for its online payment",
    ),
    (
        "wl_sales.deliveries.status.pending_acceptance",
        "the order is waiting to be accepted",
    ),
    (
        "wl_sales.deliveries.status.credit_hold",
        "the order is on credit hold",
    ),
    (
        "wl_sales.deliveries.status.fulfilled",
        "the order is fulfilled already",
    ),
    ("wl_sales.deliveries.status.closed", "the order is closed"),
    (
        "wl_sales.deliveries.status.rejected",
        "the order was rejected",
    ),
    (
        "wl_sales.deliveries.status.cancelled",
        "the order was cancelled",
    ),
    // The tasks.
    (
        "wl_sales.deliveries.not_fulfilled_title",
        "Order :number: delivery :delivery was not counted",
    ),
    (
        "wl_sales.deliveries.not_fulfilled_body",
        "The customer accepted goods of order :number on :delivery, but the order could not take them: :why. Nothing was fulfilled and nothing was invoiced for them. Look at the order and put it right.",
    ),
    (
        "wl_sales.deliveries.lost_title",
        "Order :number: delivery :delivery is lost",
    ),
    (
        "wl_sales.deliveries.lost_body",
        "The goods of order :number on :delivery are lost. Nothing is counted as fulfilled and the lines show lost. No replacement and no credit is made for you: talk to the customer, then deliver again or cancel the lines.",
    ),
    (
        "wl_sales.deliveries.closed_title",
        "Order :number is closed but delivery :delivery was reversed",
    ),
    (
        "wl_sales.deliveries.closed_body",
        "Delivery :delivery of order :number was reversed or lost after the order was closed. The order stays closed. Check what was invoiced for it and what the customer owes.",
    ),
    (
        "wl_sales.deliveries.invoice_title",
        "Order :number: an issued invoice is affected by delivery :delivery",
    ),
    (
        "wl_sales.deliveries.invoice_body",
        "Delivery :delivery of order :number was reversed or lost, and invoice :invoice was issued for it already. The invoice is not changed. Issue a credit note for the goods that were not delivered if the customer should not pay for them.",
    ),
    // The notice of a returned delivery.
    (
        "wl_sales.notices.delivery_returned_subject",
        "A delivery of order {{ number }} came back",
    ),
    (
        "wl_sales.notices.delivery_returned_body",
        "The delivery of order {{ number }} came back ({{ reason }}). The order stays open. We will deliver again or cancel what was not delivered.",
    ),
    // The fulfilment progress of a line.
    ("wl_sales.progress.label", "Delivery"),
    ("wl_sales.progress.not_dispatched", "Not dispatched"),
    ("wl_sales.progress.picked", "Picked"),
    ("wl_sales.progress.out_for_delivery", "Out for delivery"),
    ("wl_sales.progress.partly_delivered", "Partly delivered"),
    ("wl_sales.progress.delivered", "Delivered"),
    ("wl_sales.progress.lost", "Lost"),
];
