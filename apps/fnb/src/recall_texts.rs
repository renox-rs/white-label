//! The texts of the recall trace (BL-FNB-13): keys `fnb.recall.*` (toasts,
//! refusals, the notice to the customer, the owner's tasks) and
//! `fnb.screen.recall.*` (what a template reads as `tx['recall...']`).

/// Every text of the recall trace.
pub(crate) const RECALL_TEXTS: &[(&str, &str)] = &[
    // Toasts.
    (
        "fnb.recall.notified",
        "The customers are told: one notice each.",
    ),
    (
        "fnb.recall.decided",
        "The decision on the public notice is recorded.",
    ),
    // What the lines are waived with.
    (
        "fnb.recall.waive_no_contact",
        "No contact: the customer's account is closed.",
    ),
    (
        "fnb.recall.waive_no_public_notice",
        "No public notice: {{ note }}",
    ),
    // Refusals.
    (
        "fnb.recall.problem.nobody_to_tell",
        "Every customer of this recall is told or waived already.",
    ),
    (
        "fnb.recall.problem.decision",
        "Choose a public notice or no public notice.",
    ),
    (
        "fnb.recall.problem.decided",
        "The decision on the public notice is already recorded.",
    ),
    (
        "fnb.recall.problem.note",
        "Write why no public notice is published: the sales without a known customer are waived with it.",
    ),
    // The notice to the customer: no product, no lot.
    ("fnb.recall.label", "Food safety notice"),
    (
        "fnb.recall.subject",
        "Food safety notice from {{ company }}",
    ),
    (
        "fnb.recall.body",
        "{{ company }}: a supplier recalled an ingredient we used in our food. Your order {{ orders }} ({{ days }}) may be affected. Please do not eat any food left from it and contact us.",
    ),
    // The owner's tasks.
    (
        "fnb.recall.task_title",
        "Recall {{ number }}: decide the public notice and tell the aggregators",
    ),
    (
        "fnb.recall.task_body",
        "Recall {{ number }} reached sales without a known customer (aggregators and the counter). Decide whether a public notice is published and tell the aggregators; record the decision on the recall page. {{ link }}",
    ),
    (
        "fnb.recall.task_closed",
        "Decision recorded: {{ decision }}",
    ),
    (
        "fnb.recall.authority_title",
        "Recall {{ number }}: notify the authority",
    ),
    (
        "fnb.recall.authority_body",
        "Notify the authority of recall {{ number }} by hand and keep the exported recall report as the evidence. The notification is not built (open item AU-20).",
    ),
    // The screen.
    ("fnb.screen.recall.title", "Recall: customers and orders"),
    (
        "fnb.screen.recall.subtitle",
        "Who may have eaten food made from the recalled lot: the customers of direct orders with their possible orders, and the sales without a known customer per outlet and day.",
    ),
    ("fnb.screen.recall.back", "Recall"),
    ("fnb.screen.recall.notify", "Tell the customers"),
    ("fnb.screen.recall.notify_title", "Tell the customers?"),
    (
        "fnb.screen.recall.notify_message",
        "Each customer gets one message listing all their possible orders, by SMS or e-mail. A customer whose account is closed is waived.",
    ),
    ("fnb.screen.recall.public", "Public notice"),
    (
        "fnb.screen.recall.public_title",
        "Public notice and the aggregators",
    ),
    (
        "fnb.screen.recall.public_description",
        "The customers of the aggregators and of the counter are not known, so they are not messaged. Decide whether a public notice is published and tell the aggregators.",
    ),
    ("fnb.screen.recall.public_submit", "Record the decision"),
    ("fnb.screen.recall.decision", "Decision"),
    (
        "fnb.screen.recall.aggregators_told",
        "The aggregators are told",
    ),
    (
        "fnb.screen.recall.note_hint",
        "Needed when no public notice is published.",
    ),
    ("fnb.screen.recall.report", "Recall record and report"),
    (
        "fnb.screen.recall.decision_title",
        "Decision on the public notice",
    ),
    ("fnb.screen.recall.decided_at", "Decided"),
    ("fnb.screen.recall.customers", "Customers of direct orders"),
    ("fnb.screen.recall.customer", "Customer"),
    ("fnb.screen.recall.no_contact", "Account closed: no contact"),
    ("fnb.screen.recall.possible_orders", "Possible orders"),
    ("fnb.screen.recall.orders_count", "Orders"),
    ("fnb.screen.recall.no_customers", "No direct-order customer"),
    (
        "fnb.screen.recall.no_customers_text",
        "No order of a known customer was made while the lot was on hand at an outlet.",
    ),
    (
        "fnb.screen.recall.unknown",
        "Sales without a known customer",
    ),
    ("fnb.screen.recall.outlet", "Outlet"),
    ("fnb.screen.recall.day", "Day"),
    ("fnb.screen.recall.sales", "Sales"),
    ("fnb.screen.recall.channels", "Channels"),
    (
        "fnb.screen.recall.no_unknown",
        "No sale without a known customer",
    ),
    (
        "fnb.screen.recall.no_unknown_text",
        "No aggregator or counter sale was made while the lot was on hand at an outlet.",
    ),
    ("fnb.screen.recall.notices", "Notices sent"),
    ("fnb.screen.recall.sent_at", "Sent"),
    ("fnb.screen.recall.notice.pending", "To tell"),
    ("fnb.screen.recall.notice.sent", "Told"),
    ("fnb.screen.recall.notice.waived", "Waived"),
    ("fnb.screen.recall.mark.confirmed", "Confirmed"),
    ("fnb.screen.recall.mark.uncertain", "Lot uncertain"),
    ("fnb.screen.recall.mark.possible", "Possible"),
    (
        "fnb.screen.recall.decision.public_notice",
        "Public notice published, aggregators told",
    ),
    (
        "fnb.screen.recall.decision.no_public_notice",
        "No public notice",
    ),
    (
        "fnb.screen.recall.note_possible",
        "Sales consume ingredients by recipe without recording which lot went into the food, so an order is only ever possible: its outlet had the recalled lot, or a batch made from it, on hand at the time of the order.",
    ),
    (
        "fnb.screen.recall.note_authority",
        "Telling the authority is not built (open item AU-20): export the recall report and send it by hand.",
    ),
    (
        "fnb.screen.recall.note_authority_task",
        "The owner has a task to tell the authority by hand (open item AU-20): export the recall report as the evidence.",
    ),
];
