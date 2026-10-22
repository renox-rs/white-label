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
    // What the students and guardians are told when the tutor changes. No
    // reason is given.
    (
        "edtech.tutors.notice.batch_title",
        "Your tutor changes from :date",
    ),
    (
        "edtech.tutors.notice.batch_body",
        "From :date the classes of :batch (:course) are taught by another tutor. The days, the times and the room do not change.",
    ),
    (
        "edtech.tutors.notice.session_title",
        "The tutor of your class on :date changes",
    ),
    (
        "edtech.tutors.notice.session_body",
        "The class :batch · :number on :date at :time is taught by another tutor. The time and the room do not change.",
    ),
];
