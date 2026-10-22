//! The texts of a blocked payer and of the freeze of an enrolment (BL-EDU-04,
//! BL-EDU-05), in English. They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // A blocked payer or student.
    (
        "edtech.blocked.problem.withdraw",
        "This party is on hold. Only the centre manager can withdraw this enrolment.",
    ),
    ("edtech.enrolments.hold_reason", "Why it is frozen"),
    (
        "edtech.enrolments.on_hold",
        "This enrolment is on hold. The seat is kept and the centre manager decides what happens to it.",
    ),
    // The membership follows the freeze and the freeze follows the membership.
    (
        "edtech.linked.pause_reason",
        "Enrolment :enrolment is frozen",
    ),
    ("edtech.enrolments.membership", "Membership"),
    ("edtech.enrolments.membership_state.active", "Billing"),
    (
        "edtech.enrolments.membership_state.past_due",
        "Billing, a cycle is overdue",
    ),
    ("edtech.enrolments.membership_state.paused", "Paused"),
    (
        "edtech.enrolments.membership_state.suspended",
        "Suspended for non-payment",
    ),
    ("edtech.enrolments.membership_state.cancelled", "Cancelled"),
    // The package follows the freeze.
    (
        "edtech.frozen.package_reason",
        "Enrolment :enrolment was frozen for :days days",
    ),
];
