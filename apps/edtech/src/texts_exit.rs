//! The texts of the money documents of a withdrawal and of the batch `cancel`
//! and `terminate` (BL-EDU-03, BL-EDU-13, BL-EDU-12), in English. They join the
//! app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The withdrawal.
    (
        "edtech.enrolments.withdrawal_sent",
        "The withdrawal waits for another user to approve the refund.",
    ),
    (
        "edtech.enrolments.pending_approval",
        "A withdrawal of this enrolment waits for approval. The enrolment stays as it is until another user decides the refund.",
    ),
    ("edtech.enrolments.settlement", "Money"),
    ("edtech.enrolments.settled", "Settled"),
    ("edtech.enrolments.settling", "Being settled"),
    ("edtech.enrolments.not_settled", "Not finished:"),
    ("edtech.enrolments.credit_notes", "Credit notes"),
    ("edtech.enrolments.refunds_asked", "Refunds asked"),
    ("edtech.enrolments.owed_invoice", "Invoice for the amount owed"),
    ("edtech.enrolments.settle", "Finish the money steps"),
    (
        "edtech.enrolments.settle_message",
        "Repeat the steps of the withdrawal that did not finish: the credit note, the refund request and the invoice for the amount owed.",
    ),
    ("edtech.enrolments.settled_now", "The money of the withdrawal is settled."),
    ("edtech.fees.owed", "Sessions used and fees owed for :enrolment"),
    (
        "edtech.settle.credit_reason",
        "Withdrawal of :enrolment: :reason",
    ),
    (
        "edtech.settle.unpaid_reason",
        "Withdrawal of :enrolment: nothing was paid on this invoice",
    ),
];
