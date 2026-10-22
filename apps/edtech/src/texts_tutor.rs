//! The texts of the tutor eligibility guard (BL-EDU-08), in English. They join
//! the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    ("edtech.tutors.the_tutor", "The tutor"),
    ("edtech.tutors.kind.police_clearance", "police clearance"),
    (
        "edtech.tutors.kind.tutor_permit",
        "private tutor permit or freelance visa",
    ),
    (
        "edtech.tutors.problem.not_eligible",
        ":tutor cannot teach: the :kind is not valid on :dates. The first date it does not cover is :first. Renew the document or choose another tutor.",
    ),
    (
        "edtech.tutors.problem.no_employee",
        ":tutor has no employee record, so the :kind cannot be checked (first session date :first). Link the tutor to an employee or choose another tutor.",
    ),
    ("edtech.tutors.problem.more_dates", " and :count more"),
];
