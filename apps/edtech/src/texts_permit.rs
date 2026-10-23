//! The texts of the expired centre permit and course approval (BL-EDU-07), in
//! English. They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The certificate's guard.
    (
        "edtech.certificates.problem.documents",
        "The centre permit and the course approval have not been checked; issue the certificate from its page.",
    ),
    // The message of `EDU.RUL.13`.
    ("edtech.permits.kind.centre_permit", "centre permit"),
    ("edtech.permits.kind.course_approval", "course approval"),
    (
        "edtech.permits.batch_line",
        ":batch · :status · :from to :until",
    ),
    (
        "edtech.permits.no_batches",
        "No batch is open or running under it.",
    ),
    (
        "edtech.permits.expiring_title",
        "The :what of :name expires on :date",
    ),
    (
        "edtech.permits.expiring_body",
        "The :what of :name expires on :date. From that day no new batch can be opened and no certificate issued under it until it is renewed. Batches open or running: :batches. A running batch is not stopped; the centre manager decides whether it finishes.",
    ),
    (
        "edtech.permits.expired_title",
        "The :what of :name expired on :date",
    ),
    (
        "edtech.permits.expired_body",
        "The :what of :name expired on :date. No new batch can be opened and no certificate issued under it until it is renewed. Batches open or running: :batches. A running batch is not stopped; the centre manager decides whether it finishes.",
    ),
    (
        "edtech.permits.task_title",
        "Renew the :what of :name before :date",
    ),
    (
        "edtech.permits.task_body",
        "The :what of :name expires on :date. Upload the renewed document to the centre's documents; batches and certificates are blocked from that day until it is accepted.",
    ),
];
