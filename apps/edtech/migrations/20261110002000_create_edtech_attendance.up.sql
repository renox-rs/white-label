-- 10 EdTech attendance (10 section 7.1 *Session* `submit_attendance`,
-- `correct_attendance`, `EDU.ATT.01`, BL-EDU-15, BL-EDU-14): the mark of a student
-- in a session, with the notice of absence, the credit the mark costs and whether
-- it was taken from the package. A row exists once the student is on the roster of
-- the session (a notice or a guest added) or the attendance was submitted.
CREATE TABLE "edtech_attendance" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES "edtech_sessions" (id),
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    student_party_id INTEGER NOT NULL,
    mark TEXT NOT NULL DEFAULT 'pending',
    notice_at TEXT,
    notice_by INTEGER,
    excuse_reason TEXT,
    excused_by INTEGER,
    credit_due BOOLEAN NOT NULL DEFAULT FALSE,
    consumed BOOLEAN NOT NULL DEFAULT FALSE,
    credit_note TEXT,
    alert_sent_at TEXT,
    marked_by INTEGER,
    marked_at TEXT,
    corrected_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_attendance_session_id_enrolment_id" ON "edtech_attendance" (session_id, enrolment_id);
CREATE INDEX "edtech_attendance_enrolment_id" ON "edtech_attendance" (enrolment_id);
