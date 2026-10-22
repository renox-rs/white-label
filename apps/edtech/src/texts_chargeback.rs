//! The texts of the task for a payment charged back (BL-EDU-06), in English.
//! They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The counsellor's task.
    (
        "edtech.chargeback.task_title",
        "Collect or withdraw: a payment for :enrolment was charged back",
    ),
    (
        "edtech.chargeback.task_body",
        "A payment for :enrolment (student :student) was reversed and the invoice reopened. To collect: :amount. Payer: :payer. Invoices: :invoices. Send payment link: :collect. Withdraw the student: :withdraw. The enrolment itself is unchanged.",
    ),
    (
        "edtech.chargeback.update_comment",
        "Still to collect on :enrolment: :amount (:invoices).",
    ),
    (
        "edtech.chargeback.closed_note",
        "Paid again: nothing is left to collect on :enrolment.",
    ),
    (
        "edtech.chargeback.withdrawn_note",
        "The student was withdrawn; the credit note and the refund take what was actually paid.",
    ),
    // The payment link.
    (
        "edtech.chargeback.problem.nothing",
        "Nothing is waiting to be collected on this enrolment.",
    ),
    (
        "edtech.chargeback.problem.no_link",
        "Online payment is not available. Ask the payer to pay at the centre.",
    ),
    ("edtech.chargeback.link_title", "Payment for :enrolment"),
    (
        "edtech.chargeback.link_body",
        "A payment for :enrolment was reversed by the bank. Please pay :amount again: :link",
    ),
    // The page.
    ("edtech.chargeback.title", "Collect a reversed payment"),
    (
        "edtech.chargeback.subtitle",
        "A card payment was charged back and the invoice is open again. The enrolment is unchanged: collect the money again or withdraw the student.",
    ),
    ("edtech.chargeback.amount", "To collect"),
    ("edtech.chargeback.payer", "Payer"),
    ("edtech.chargeback.invoices", "Invoices"),
    ("edtech.chargeback.invoice", "Invoice"),
    ("edtech.chargeback.reopened_on", "Reopened"),
    ("edtech.chargeback.balance", "Open balance"),
    ("edtech.chargeback.reason", "Reversed because"),
    ("edtech.chargeback.send_link", "Send payment link"),
    (
        "edtech.chargeback.send_link_message",
        "Make a payment link for the amount to collect and send it to the payer.",
    ),
    ("edtech.chargeback.link_sent", "The payment link was sent."),
    ("edtech.chargeback.withdraw", "Withdraw the student"),
    (
        "edtech.chargeback.empty",
        "Nothing is waiting to be collected",
    ),
    (
        "edtech.chargeback.alert",
        "A payment for this enrolment was charged back. The money is waiting to be collected.",
    ),
    ("edtech.chargeback.open", "Collect it"),
    ("edtech.chargeback.to_collect", "To collect now"),
];
