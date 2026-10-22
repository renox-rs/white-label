//! The texts of the tutors who will not be eligible or available (BL-EDU-10,
//! `EDU.RUL.12`), in English. They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The reasons a session is flagged.
    ("edtech.alerts.reason.document", "the :kind is not valid"),
    (
        "edtech.alerts.reason.notice",
        "notice was given to the tutor or by them",
    ),
    (
        "edtech.alerts.reason.block",
        "the tutor is marked unavailable",
    ),
    ("edtech.alerts.reason.leave", "the tutor is on leave"),
    // What a trigger says.
    ("edtech.alerts.detail.document", ":kind valid to :date"),
    ("edtech.alerts.detail.notice", "last working day :date"),
    (
        "edtech.alerts.detail.block",
        "not available from :from to :until",
    ),
    (
        "edtech.alerts.detail.leave",
        "on leave from :from to :until",
    ),
    // The coordinator's task.
    (
        "edtech.alerts.task_title",
        "Change the tutor or reschedule: :tutor, :detail",
    ),
    (
        "edtech.alerts.task_body",
        ":tutor: :detail. These sessions are flagged. Change the tutor or reschedule each of them before it starts; nothing is changed by itself.\n:sessions",
    ),
    (
        "edtech.alerts.task_bookings",
        "\n\nOne-to-one bookings (flagged and escalated by the booking conflicts of the schedule, not here):\n:bookings",
    ),
    (
        "edtech.alerts.task_update",
        "More sessions of this tutor are now inside the window:\n:lines",
    ),
    (
        "edtech.alerts.session_line",
        ":batch · :number · :date at :time",
    ),
    ("edtech.alerts.booking_line", ":booking on :date"),
    (
        "edtech.alerts.resolved",
        "Nothing is left to change: the document was renewed or the period is over.",
    ),
    ("edtech.alerts.withdrawn", "The notice was withdrawn."),
    // The message to the centre manager a day before a flagged session.
    (
        "edtech.alerts.escalation_title",
        "Session :session on :date is still flagged",
    ),
    (
        "edtech.alerts.escalation_body",
        "The session :session on :date at :time starts within a day and its tutor is not available or not eligible (:reason). Change the tutor or move the session before it starts.",
    ),
    // The flag on the pages.
    (
        "edtech.sessions.flag",
        "Tutor not available or not eligible",
    ),
    ("edtech.sessions.flag_column", "Flag"),
    // The documents flagged at the attendance (`session.submit_attendance`).
    ("edtech.sessions.doc_flags", "Documents flagged"),
    ("edtech.sessions.doc_flag", "Flag"),
    ("edtech.sessions.flag_open", "Open"),
    ("edtech.sessions.flag_closed_status", "Closed"),
    ("edtech.sessions.flag_reviewed_by", "Closed by"),
    ("edtech.sessions.flag_note", "Note"),
    ("edtech.sessions.close_flag", "Close the flag"),
    (
        "edtech.sessions.close_flag_description",
        "The attendance stays as it was submitted. The flag stays in the history with your note and your name.",
    ),
    ("edtech.sessions.flag_closed", "Flag closed."),
    (
        "edtech.sessions.problem.flag_note",
        "A note is needed to close the flag.",
    ),
];
