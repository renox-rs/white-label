//! The texts of the uncollected vehicles (`AUT.RUL.18`, BL-AUT-05), in
//! English. They join the app's other texts in [`crate::texts`].

/// Every key of this part, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    (
        "auto.storage.reason_ready",
        "your vehicle has been ready for :days days",
    ),
    (
        "auto.storage.reason_declined",
        "the job was declined :days days ago",
    ),
    (
        "auto.storage.reason_cancelled",
        "the job was cancelled :days days ago",
    ),
    (
        "auto.storage.fee_sms",
        " Storage of :amount a day is charged from now.",
    ),
    (
        "auto.storage.task_call",
        "Call :customer: the vehicle of :job has waited :days days to be collected",
    ),
    (
        "auto.storage.manager_title",
        ":job: vehicle not collected for :days days, legal next steps open (L-36)",
    ),
    (
        "auto.storage.manager_body",
        "The vehicle of :job has not been collected for :days days. What the workshop may do with an abandoned vehicle is open with the lawyer (L-36); the app offers no sale, disposal or lien.",
    ),
    (
        "auto.storage.invoice_line",
        "Storage of the vehicle (:days days at :rate a day)",
    ),
    ("auto.storage.title", "Storage"),
    ("auto.storage.days", "Days of storage"),
    ("auto.storage.invoice", "Storage invoice"),
    ("auto.storage.first", "First reminder"),
    ("auto.storage.second", "Second reminder"),
    ("auto.storage.manager", "Branch manager told"),
];
