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
