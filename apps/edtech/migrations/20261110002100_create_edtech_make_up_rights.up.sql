-- 10 EdTech make-up right (10 section 4.2 *A student is absent*, `EDU.MKP.01`,
-- BL-EDU-15): one per excused absence, valid until the enrolment ends. Booking
-- the make-up session itself is `EDU.MKP.01`; `used_session_id` is where it was
-- taken, `voided_at` when a correction took the excused absence away.
CREATE TABLE "edtech_make_up_rights" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    attendance_id INTEGER NOT NULL REFERENCES "edtech_attendance" (id),
    session_id INTEGER NOT NULL,
    used_session_id INTEGER,
    voided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_make_up_rights_attendance_id" ON "edtech_make_up_rights" (attendance_id);
CREATE INDEX "edtech_make_up_rights_enrolment_id" ON "edtech_make_up_rights" (enrolment_id);
