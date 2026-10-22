//! The texts of the lapse of a seat hold and of the notices of the vertical's
//! negative flows (BL-EDU-01), in English. They join the app's other texts in
//! [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The notification types: a title and a body the caller words.
    ("edtech.notice.subject", "{{ title }}"),
    ("edtech.notice.body", "{{ body }}"),
    // The lapse.
    ("edtech.lapse.reason", "No payment within the seat hold"),
    (
        "edtech.lapse.problem.no_facts",
        "The enrolment lapses only after the payments were looked at.",
    ),
    (
        "edtech.lapse.problem.paid",
        "A payment arrived, so the enrolment does not lapse.",
    ),
    (
        "edtech.lapse.problem.pending",
        "A payment is still pending at the gateway, so the enrolment does not lapse yet.",
    ),
    (
        "edtech.lapse.problem.not_due",
        "The seat is still held: the hold has not ended.",
    ),
    (
        "edtech.lapse.family_title",
        "Your seat in :batch was released",
    ),
    (
        "edtech.lapse.family_body",
        "The first payment for :enrolment did not arrive within :hours hours, so the seat was released and the payment link no longer works. Contact the centre to enrol again.",
    ),
    ("edtech.lapse.staff_title", "Enrolment :enrolment lapsed"),
    (
        "edtech.lapse.staff_body",
        ":student's seat in :batch was released: no payment arrived within :hours hours.",
    ),
    ("edtech.lapse.retry_title", "Your payment did not go through"),
    (
        "edtech.lapse.retry_body",
        "The payment for :enrolment failed and your seat is still held until :until. Pay with this new link: :link",
    ),
    ("edtech.lapse.refund_title", "A late payment was refunded"),
    (
        "edtech.lapse.refund_body",
        ":amount paid for :enrolment arrived after the seat hold ended and was refunded to the card. Enrol the student again and send the new payment link.",
    ),
    // The enrolment page.
    ("edtech.enrolments.paid", "Paid so far"),
    ("edtech.enrolments.balance", "Balance to pay"),
    ("edtech.enrolments.payment_link", "Payment link"),
];
