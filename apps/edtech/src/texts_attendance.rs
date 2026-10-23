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
        "Mark each student present, late, absent or excused.",
    ),
    (
        "edtech.attendance.problem.time",
        "Give the time as year-month-day and hour:minute, Dubai time, for example 2026-11-09 18:00.",
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
    (
        "edtech.attendance.empty",
        "No student is expected in this session.",
    ),
    ("edtech.attendance.submit", "Submit attendance"),
    (
        "edtech.attendance.submit_description",
        "Mark each student. A student excused by a notice stays excused unless you mark them present.",
    ),
    ("edtech.attendance.field", "Mark of :student"),
    ("edtech.attendance.add", "Add a student"),
    (
        "edtech.attendance.add_description",
        "A student with a package takes this session; a credit is used by the mark.",
    ),
    ("edtech.attendance.added", "Student added."),
    ("edtech.attendance.enrolment_number", "Enrolment"),
    (
        "edtech.attendance.record_notice",
        "Record a notice of absence",
    ),
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
    (
        "edtech.credits.note.unpaid",
        "the package has an unpaid balance",
    ),
    (
        "edtech.credits.task_done",
        "The credit was taken from a package.",
    ),
    // The correction of the attendance (BL-EDU-14).
    ("edtech.corrections.action", "Correct attendance"),
    (
        "edtech.corrections.description",
        "Change the marks that were wrong and say why. Credits, make-up rights and the certificates that depend on them follow.",
    ),
    ("edtech.corrections.reason", "Reason"),
    ("edtech.corrections.done", "Attendance corrected."),
    (
        "edtech.corrections.problem.not_held",
        "Only the attendance of a session that took place can be corrected.",
    ),
    (
        "edtech.corrections.problem.reason",
        "A reason is needed to correct the attendance.",
    ),
    (
        "edtech.corrections.problem.window_over",
        "The tutor can correct the attendance until 24 hours after the session ends. Ask the academic coordinator, who corrects it with a reason.",
    ),
    (
        "edtech.corrections.problem.nothing",
        "The marks are as they were: there is nothing to correct.",
    ),
    (
        "edtech.corrections.guardian_title",
        "The attendance of :student on :date was corrected",
    ),
    (
        "edtech.corrections.guardian_body",
        "The attendance of :student on :date was corrected and is now: :mark. An earlier message about sessions missed in a row does not apply to that day any more.",
    ),
    // The certificate whose criteria a correction no longer meets.
    (
        "edtech.completion.task_title",
        "Reissue or revoke certificate :certificate: attendance is now :percent%",
    ),
    (
        "edtech.completion.task_body",
        "After a correction of the attendance, :student has :percent% in the course, below the :threshold% the certificate :certificate asks for. The certificate is unchanged. Reissue it if the result still meets the criteria, or ask the company admin to revoke it.",
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
