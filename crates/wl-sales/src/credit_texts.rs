//! The texts of the credit control (BL-SAL-10 to BL-SAL-12): the credit hold,
//! the account hold and its clearing. Read through [`crate::texts`].

/// Every key of the credit control, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    (
        "wl_sales.credit.reason.limit",
        "The customer would be over the credit limit",
    ),
    (
        "wl_sales.credit.reason.overdue",
        "An invoice is overdue beyond the grace days",
    ),
    (
        "wl_sales.credit.reason.account_on_hold",
        "The customer account is on hold",
    ),
    (
        "wl_sales.credit.problem.limit",
        "The credit limit cannot be negative.",
    ),
    (
        "wl_sales.credit.problem.days",
        "The payment terms and the grace days cannot be negative.",
    ),
    (
        "wl_sales.credit.problem.group",
        "Choose a credit group that exists.",
    ),
    (
        "wl_sales.orders.problem.not_on_account",
        "Only an order on account goes on credit hold.",
    ),
    (
        "wl_sales.orders.problem.credit_not_checked",
        "Accept the order from its page: its customer's credit is checked first.",
    ),
    ("wl_sales.nav.credit_accounts", "Credit accounts"),
    ("wl_sales.credit.title", "Credit accounts"),
    (
        "wl_sales.credit.subtitle",
        "What each customer may owe on account, and what it owes now.",
    ),
    ("wl_sales.credit.empty", "No customer accounts"),
    ("wl_sales.credit.new", "New customer account"),
    (
        "wl_sales.credit.new_description",
        "Credit limit and payment terms of a customer who buys on account.",
    ),
    ("wl_sales.credit.edit", "Change the terms"),
    ("wl_sales.credit.save", "Save"),
    ("wl_sales.credit.saved", "Customer account saved."),
    ("wl_sales.credit.limit", "Credit limit (AED)"),
    ("wl_sales.credit.no_limit", "No limit"),
    ("wl_sales.credit.terms", "Payment terms"),
    ("wl_sales.credit.days", ":days days"),
    ("wl_sales.credit.grace", "Grace days past terms"),
    ("wl_sales.credit.sales_rep", "Sales rep (user id)"),
    ("wl_sales.credit.group", "Credit group"),
    ("wl_sales.credit.no_group", "None"),
    ("wl_sales.credit.owes", "Owes now"),
    ("wl_sales.credit.on_hold", "Account on hold"),
    ("wl_sales.credit.not_on_hold", "Account in good standing"),
    ("wl_sales.credit.account_subtitle", "Credit account"),
    ("wl_sales.credit.terms_title", "Terms"),
    ("wl_sales.credit.exposure_title", "What the customer owes"),
    ("wl_sales.credit.invoices", "Open invoices"),
    ("wl_sales.credit.cheques", "Cheques not cleared"),
    ("wl_sales.credit.orders", "Confirmed orders not invoiced"),
    ("wl_sales.credit.overdue", "Overdue beyond the grace days"),
    ("wl_sales.credit.available", "Credit left"),
    ("wl_sales.credit.held_title", "Orders on credit hold"),
    ("wl_sales.credit.no_held", "No orders on credit hold"),
    ("wl_sales.credit.value", "Still to deliver"),
    ("wl_sales.credit.hold_reason", "Why"),
    ("wl_sales.credit.returns_to", "Returns to"),
    ("wl_sales.credit.held_on", "On hold since"),
    ("wl_sales.credit.hold_title", "On credit hold"),
    (
        "wl_sales.credit.hold_goods_reserved",
        "The goods stay reserved. No dispatch can leave until a credit releaser releases the order.",
    ),
    ("wl_sales.orders.all", "All orders"),
    ("wl_sales.orders.on_credit_hold", "Orders on credit hold"),
    ("wl_sales.orders.accept", "Accept the order"),
    (
        "wl_sales.orders.accept_message",
        "The customer's credit is checked first; an order that fails it goes on credit hold.",
    ),
    ("wl_sales.orders.accepted", "Order accepted."),
    (
        "wl_sales.orders.accepted_held",
        "Order accepted, but it is on credit hold.",
    ),
    (
        "wl_sales.notices.order_credit_hold_subject",
        "Order {{ number }} is on credit hold",
    ),
    (
        "wl_sales.notices.order_credit_hold_body",
        "Order {{ number }} for {{ customer }} is on credit hold: {{ reason }}. The customer owes {{ open_balance }} with this order, {{ overdue }} of it overdue; the limit is {{ limit }}. The goods stay reserved until a credit releaser releases the order.",
    ),
];
