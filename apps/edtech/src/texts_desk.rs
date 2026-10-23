//! The texts of the fee collected at the front desk ([`crate::desk`]), in English.
//! They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("edtech.batches.tutor_pick", "Tutor"),
    ("edtech.batches.room_pick", "Room"),
    ("edtech.transfer.target_pick", "Move to the batch"),
    ("edtech.verify.code", "Verification code"),
    (
        "edtech.verify.code_hint",
        "The code printed on the certificate, for example 3WDJL0ICIJ",
    ),
    ("edtech.verify.check", "Check the certificate"),
    (
        "edtech.sessions.problem.past",
        "The session cannot move to a time that has already passed. Give a later date or time.",
    ),
    ("edtech.desk.action", "Record a payment"),
    (
        "edtech.desk.description",
        "The student or the guardian pays at the desk. The money is recorded and set against the open invoices of this enrolment; the first invoice paid in full activates it.",
    ),
    ("edtech.desk.method", "How it was paid"),
    ("edtech.desk.method.cash", "Cash"),
    ("edtech.desk.method.card", "Card"),
    ("edtech.desk.method.transfer", "Bank transfer"),
    ("edtech.desk.amount", "Amount (AED)"),
    (
        "edtech.desk.amount_hint",
        "Up to what is due: for example 2520 or 2520.50",
    ),
    ("edtech.desk.reference", "Reference"),
    (
        "edtech.desk.reference_hint",
        "The bank reference of a transfer, or the card slip number",
    ),
    ("edtech.desk.submit", "Record the payment"),
    ("edtech.desk.done", "The payment is recorded."),
    ("edtech.desk.due", "Due now"),
    (
        "edtech.desk.problem.closed",
        "This enrolment has ended; it takes no payment.",
    ),
    (
        "edtech.desk.problem.method",
        "Choose how it was paid: cash, card or bank transfer.",
    ),
    (
        "edtech.desk.problem.nothing_due",
        "Nothing is due on this enrolment.",
    ),
    (
        "edtech.desk.problem.amount",
        "The amount must be more than 0, for example 2520.50.",
    ),
    (
        "edtech.desk.problem.too_much",
        "The amount is more than what is due, :due.",
    ),
    (
        "edtech.desk.problem.reference",
        "A bank transfer needs its reference.",
    ),
];
