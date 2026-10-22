//! The texts of the attendance, the absence policy and the credits (BL-EDU-15,
//! BL-EDU-14), in English. They join the app's other texts in [`crate::texts`].

/// Every key of these parts, with its English text.
pub(crate) const TEXTS: &[(&str, &str)] = &[
    // The marks.
    ("edtech.attendance.mark.pending", "Not marked"),
    ("edtech.attendance.mark.present", "Present"),
    ("edtech.attendance.mark.late", "Late (counts as present)"),
    ("edtech.attendance.mark.absent", "Absent"),
    ("edtech.attendance.mark.excused", "Excused"),
    // The refusals.
    (
        "edtech.attendance.problem.session_over",
        "The session is not scheduled any more: notices and guests are taken until it starts.",
    ),
    (
        "edtech.attendance.problem.not_active",
        "Only an active enrolment can take a session.",
    ),
    (
        "edtech.attendance.problem.already_on",
        "The student is on the roster of this session already.",
    ),
    (
        "edtech.attendance.problem.notice_after_start",
        "A notice given after the session started is an absence, not a notice.",
    ),
    (
        "edtech.attendance.problem.not_on_roster",
        "The student is not on the roster of this session.",
    ),
    (
        "edtech.attendance.problem.no_notice",
        "There is no late notice of absence to excuse for this student.",
    ),
    (
        "edtech.attendance.problem.reason",
        "A reason is needed to excuse a late notice.",
    ),
    (
        "edtech.attendance.problem.excuse_needs_notice",
        "An absence is excused only by a notice given :hours hours before the session, or by the coordinator with a reason.",
    ),
    (
        "edtech.attendance.problem.unmarked",
        "Every student must be marked: :students.",
    ),
    (
        "edtech.attendance.problem.not_all_marked",
        "Every student of the session must be marked.",
    ),
    (
        "edtech.attendance.problem.mark",
        "mark: present, late, absent or excused",
    ),
    (
        "edtech.attendance.problem.time",
        "time: YYYY-MM-DD HH:MM, Dubai time",
    ),
    // The page.
    ("edtech.attendance.title", "Students"),
    ("edtech.attendance.student", "Student"),
    ("edtech.attendance.enrolment", "Enrolment"),
    ("edtech.attendance.mark", "Mark"),
    ("edtech.attendance.notice", "Notice of absence"),
    ("edtech.attendance.credit", "Credit"),
    ("edtech.attendance.credit_used", "Used"),
    ("edtech.attendance.credit_not_used", "Not used"),
    ("edtech.attendance.credit_waiting", "Waiting for a credit"),
    ("edtech.attendance.empty", "No student is expected in this session."),
    ("edtech.attendance.submit", "Submit attendance"),
    (
        "edtech.attendance.submit_description",
        "Mark each student. A student excused by a notice stays excused unless you mark them present.",
    ),
    (
        "edtech.attendance.field",
        "Mark of :student",
    ),
    ("edtech.attendance.add", "Add a student"),
    (
        "edtech.attendance.add_description",
        "A student with a package takes this session; a credit is used by the mark.",
    ),
    ("edtech.attendance.added", "Student added."),
    ("edtech.attendance.enrolment_number", "Enrolment"),
    ("edtech.attendance.record_notice", "Record a notice of absence"),
    (
        "edtech.attendance.record_notice_description",
        "The notice and its time. 24 hours or more before the session the student is excused; later the absence stays unexcused unless the coordinator excuses it.",
    ),
    ("edtech.attendance.given_at", "Given at (YYYY-MM-DD HH:MM)"),
    ("edtech.attendance.notice_recorded", "Notice recorded."),
    ("edtech.attendance.excuse", "Excuse a late notice"),
    (
        "edtech.attendance.excuse_description",
        "The absence is excused with your reason: no credit is used and the student earns a make-up right.",
    ),
    ("edtech.attendance.excuse_reason", "Reason"),
    ("edtech.attendance.excused", "Absence excused."),
    ("edtech.attendance.make_up_rights", "Make-up rights"),
    // The credits and the task to sell more.
    (
        "edtech.credits.task_title",
        "Sell credits to :student: no :item credit left",
    ),
    (
        "edtech.credits.task_body",
        ":student (:enrolment) attended a session and the package of :item has no credit left to use (:reason). The attendance is saved. Sell a new package; the credit is taken from it when it is paid.",
    ),
    ("edtech.credits.note.no_credit", "no credit left"),
    ("edtech.credits.note.unpaid", "the package has an unpaid balance"),
    (
        "edtech.credits.task_done",
        "The credit was taken from a package.",
    ),
    // The alert of `EDU.RUL.02`.
    (
        "edtech.absences.title",
        ":student has missed :count sessions in a row",
    ),
    (
        "edtech.absences.body",
        ":student (:enrolment) has been absent :count sessions in a row, the last on :date. Please get in touch with the centre.",
    ),
];
