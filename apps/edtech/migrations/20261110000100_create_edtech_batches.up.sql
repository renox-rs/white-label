-- 10 EdTech batch (`EDU.BAT.01`, 10 section 7.1 *Batch*): one run of a course
-- with capacity, dates, weekly pattern, tutor and room (wl-scheduling
-- resources, named by id). `seats_taken` is the plain seat count that
-- `seats::take` moves in one statement (BL-EDU-02 replaces it with the count
-- of the enrolments under a lock on this row).
CREATE TABLE "edtech_batches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    course_id INTEGER NOT NULL REFERENCES "edtech_courses" (id),
    campus_type TEXT,
    campus_id INTEGER,
    capacity INTEGER NOT NULL,
    seats_taken INTEGER NOT NULL DEFAULT 0,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    weekdays TEXT NOT NULL DEFAULT '[]',
    start_time TEXT NOT NULL,
    duration_minutes INTEGER NOT NULL,
    tutor_resource_id INTEGER,
    tutor_employee_id INTEGER,
    room_resource_id INTEGER,
    room_limit INTEGER,
    cancel_reason TEXT,
    termination_reason TEXT,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "edtech_batches_course_id" ON "edtech_batches" (course_id);
CREATE INDEX "edtech_batches_status" ON "edtech_batches" (status);
