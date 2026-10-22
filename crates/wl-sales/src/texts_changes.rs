//! The texts of the changes to a placed order (BL-SAL-05, BL-SAL-06,
//! BL-SAL-08, BL-SAL-09, shared with BL-SAL-07), in English, listed apart from
//! the rest of the package's texts. [`crate::texts`] reads this list as well,
//! so an app translates one the same way: by putting the key in its
//! `resources/lang/{locale}.json`.

/// Every key of the changes, with its English text.
pub(crate) const CHANGE_TEXTS: &[(&str, &str)] = &[
    // Amending an order (BL-SAL-05).
    ("wl_sales.amend.title", "Amend the order"),
    (
        "wl_sales.amend.description",
        "A new revision is made; the revision in force is kept and can be read but not changed. Give the new quantity, price and discount of each line you change.",
    ),
    ("wl_sales.amend.action", "Amend"),
    ("wl_sales.amend.submit", "Save as a new revision"),
    ("wl_sales.amend.reason", "Reason for the amendment"),
    ("wl_sales.amend.revision", "Revision"),
    ("wl_sales.amend.current", "In force"),
    ("wl_sales.amend.line", "Line"),
    ("wl_sales.amend.quantity", "New quantity"),
    ("wl_sales.amend.price", "New price"),
    ("wl_sales.amend.discount", "New discount"),
    ("wl_sales.amend.add_title", "Add a line"),
    ("wl_sales.amend.add_item", "Item code"),
    ("wl_sales.amend.add_quantity", "Quantity"),
    ("wl_sales.amend.add_price", "Price"),
    (
        "wl_sales.amend.done",
        "The order is now at revision :revision.",
    ),
    (
        "wl_sales.amend.waiting",
        "A manager must approve this amendment first. The revision in force stays until then.",
    ),
    (
        "wl_sales.amend.aggregator",
        "This order comes from an aggregator and is changed on the aggregator.",
    ),
    (
        "wl_sales.amend.use_cancel_line",
        "To take quantity off a line whose preparation started, cancel the line: it asks what happens to the goods.",
    ),
    (
        "wl_sales.amend.problem.state",
        "An order in this state cannot be amended.",
    ),
    (
        "wl_sales.amend.problem.channel",
        "Orders of this channel are changed on the channel's own system.",
    ),
    ("wl_sales.amend.problem.reason", "Give the reason for the amendment."),
    (
        "wl_sales.amend.problem.moved",
        "The order was changed since you opened it. Reload it and try again.",
    ),
    (
        "wl_sales.amend.problem.negative",
        "Line :position: give a quantity, price and discount of zero or more.",
    ),
    (
        "wl_sales.amend.problem.below_fulfilled",
        "Line :position cannot go below what was already handed over or cancelled.",
    ),
    (
        "wl_sales.amend.problem.started_reduced",
        "Line :position is already being prepared: cancel the line to take quantity off it.",
    ),
    ("wl_sales.amend.problem.nothing", "Nothing was changed."),
    (
        "wl_sales.amend.problem.empty",
        "The amendment would leave nothing on the order: cancel the order instead.",
    ),
    // Revisions kept read-only.
    ("wl_sales.amend.revision_title", "Order :number, revision :revision"),
    ("wl_sales.revisions.title", "Earlier revisions"),
    ("wl_sales.revisions.view", "Read"),
    ("wl_sales.revisions.when", "Replaced on"),
    (
        "wl_sales.revisions.read_only",
        "This revision is kept as it was and cannot be changed.",
    ),
    ("wl_sales.revisions.back", "Back to the order"),
    ("wl_sales.revisions.empty", "No earlier revision"),
    // What every change shares.
    (
        "wl_sales.changes.pending",
        "A change of this kind already waits for a manager.",
    ),
    (
        "wl_sales.changes.no_policy",
        "This change needs a manager, but no approval policy exists for it.",
    ),
    (
        "wl_sales.changes.waiting_alert",
        "A manager must approve a change to this order before it applies.",
    ),
    // Cancelling a line or the remaining lines (BL-SAL-08, BL-SAL-09).
    ("wl_sales.cancelling.started", "Preparation started"),
    ("wl_sales.cancelling.outcome_label", "What happens to the goods"),
    (
        "wl_sales.cancelling.outcome_none",
        "Not needed: preparation has not started",
    ),
    ("wl_sales.cancelling.outcome.return_to_stock", "Return to stock"),
    ("wl_sales.cancelling.outcome.finish_to_stock", "Finish to stock"),
    ("wl_sales.cancelling.outcome.scrap", "Scrap"),
    ("wl_sales.cancelling.fee", "Cancellation fee"),
    (
        "wl_sales.cancelling.fee_hint",
        "An amount, such as 8.00. Leave empty for the default of the settings.",
    ),
    ("wl_sales.cancelling.waive_fee", "Waive the fee"),
    (
        "wl_sales.cancelling.approval_hint",
        "A line whose preparation started needs an outcome and a manager's approval.",
    ),
    (
        "wl_sales.cancelling.line_cancelled",
        "The quantity is cancelled.",
    ),
    (
        "wl_sales.cancelling.waiting",
        "A manager must approve this cancellation first. Nothing changes until then.",
    ),
    ("wl_sales.cancelling.remaining", "Cancel the remaining lines"),
    (
        "wl_sales.cancelling.remaining_description",
        "Every line that is still open is cancelled and the order is complete. Give the outcome of each line whose preparation started.",
    ),
    (
        "wl_sales.cancelling.remaining_done",
        "The remaining lines are cancelled.",
    ),
    (
        "wl_sales.cancelling.remaining_blocked",
        "Cancel the remaining lines is offered once something was handed over. Nothing was yet: cancel the order instead.",
    ),
    ("wl_sales.cancelling.open", "Open"),
    (
        "wl_sales.cancelling.problem.state",
        "An order in this state cannot be cancelled this way.",
    ),
    ("wl_sales.cancelling.problem.reason", "Give the reason."),
    (
        "wl_sales.cancelling.problem.nothing_open",
        "No line has quantity left open.",
    ),
    (
        "wl_sales.cancelling.problem.outcome",
        "Line :position is being prepared: choose what happens to the goods.",
    ),
    (
        "wl_sales.cancelling.problem.outcome_not_offered",
        "Line :position: that outcome is not offered here.",
    ),
    (
        "wl_sales.cancelling.problem.fee",
        "Line :position: the fee cannot be negative or above the line's prepaid share.",
    ),
    (
        "wl_sales.cancelling.problem.moved",
        "The order changed while the request waited: ask again.",
    ),
    // The credit check an amendment re-runs.
    ("wl_sales.credit.hold_reason", "Credit limit exceeded by an amendment."),
    ("wl_sales.credit.task_title", "Order :number is on credit hold"),
    (
        "wl_sales.credit.task_body",
        "An amendment raised order :number to :total and the customer's credit limit is exceeded. Review the account and release the order or ask for payment.",
    ),
    // The money of a change (BL-SAL-06 and the cancellations).
    ("wl_sales.settle.title", "Money of the changes"),
    ("wl_sales.settle.kind_label", "Step"),
    ("wl_sales.settle.amount", "Amount"),
    ("wl_sales.settle.status_label", "Status"),
    ("wl_sales.settle.link_path", "Payment link"),
    ("wl_sales.settle.due", "Valid until"),
    ("wl_sales.settle.note", "Note"),
    ("wl_sales.settle.empty", "No money step yet"),
    ("wl_sales.settle.kind.link", "Payment link for an increase"),
    ("wl_sales.settle.kind.refund", "Refund"),
    ("wl_sales.settle.kind.credit_note", "Credit note"),
    ("wl_sales.settle.kind.extra_invoice", "Extra invoice"),
    ("wl_sales.settle.kind.fee_invoice", "Cancellation fee invoice"),
    ("wl_sales.settle.kind.draft_updated", "Draft invoice updated"),
    ("wl_sales.settle.status.open", "Waiting for the customer"),
    ("wl_sales.settle.status.paid", "Paid"),
    ("wl_sales.settle.status.expired", "Expired unpaid"),
    ("wl_sales.settle.status.resent", "Replaced by a new link"),
    ("wl_sales.settle.status.reverted", "Amended back"),
    ("wl_sales.settle.status.requested", "Requested"),
    ("wl_sales.settle.status.done", "Done"),
    ("wl_sales.settle.status.failed", "Failed: a person must act"),
    ("wl_sales.settle.held", "Held until the payment link is paid"),
    (
        "wl_sales.settle.expired_hint",
        "The payment link for the increase expired unpaid. The added quantity stays reserved but is not released. Resend the link or amend the order back.",
    ),
    ("wl_sales.settle.resend", "Resend the link"),
    (
        "wl_sales.settle.resend_message",
        "A new link for the same amount is sent to the customer.",
    ),
    ("wl_sales.settle.resent", "A new payment link was sent."),
    ("wl_sales.settle.amend_back", "Amend back"),
    (
        "wl_sales.settle.amend_back_message",
        "A new revision without the increase is made. Nothing is charged or refunded.",
    ),
    ("wl_sales.settle.amended_back", "The order is amended back."),
    ("wl_sales.settle.reason_amended", "order amended"),
    ("wl_sales.settle.reason_cancelled", "order line cancelled"),
    (
        "wl_sales.settle.amend_back_reason",
        "The payment link of the increase expired unpaid.",
    ),
    ("wl_sales.settle.extra_line", ":description (amendment, revision :revision)"),
    ("wl_sales.settle.fee_line", "Cancellation fee for order :number"),
    (
        "wl_sales.settle.no_gateway",
        "No payment link could be made: no payment gateway is available.",
    ),
    (
        "wl_sales.settle.expired_task_title",
        "Payment link expired for order :number",
    ),
    (
        "wl_sales.settle.expired_task_body",
        "The payment link of :amount for the increase of order :number (revision :revision) was not paid in time. The added quantity is not released. Open the order and resend the link or amend the order back.",
    ),
    (
        "wl_sales.settle.problem.no_expired_link",
        "No payment link of an amendment has expired on this order.",
    ),
    (
        "wl_sales.settle.problem.order_closed",
        "The order is closed: a new link cannot be sent.",
    ),
    // Notices.
    (
        "wl_sales.notices.order_revised_subject",
        "Your order has been changed",
    ),
    (
        "wl_sales.notices.order_revised_body",
        "Order {{ number }} is now at revision {{ revision }}. The total was {{ before }} and is now {{ total }}.",
    ),
    (
        "wl_sales.notices.order_amend_link_subject",
        "Please pay the difference for your order",
    ),
    (
        "wl_sales.notices.order_amend_link_body",
        "Your change to order {{ number }} adds {{ amount }}. Pay it at {{ link }} to have the added items prepared.",
    ),
    (
        "wl_sales.notices.order_lines_cancelled_subject",
        "Part of your order was cancelled",
    ),
    (
        "wl_sales.notices.order_lines_cancelled_body",
        "{{ lines }} line(s) of order {{ number }} were cancelled, worth {{ value }}. Any cancellation fee is {{ fee }}; the rest of what you paid is being refunded.",
    ),
    (
        "wl_sales.notices.order_credit_hold_subject",
        "Order on credit hold",
    ),
    (
        "wl_sales.notices.order_credit_hold_body",
        "Order {{ number }} ({{ total }}) is on credit hold after an amendment: the customer's exposure would be {{ exposure }}.",
    ),
];
