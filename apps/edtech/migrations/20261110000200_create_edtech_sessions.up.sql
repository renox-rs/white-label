-- 10 EdTech session (`EDU.BAT.01`, 10 section 7.1 *Session*): one class
-- meeting of a batch, generated from its weekly pattern when the batch opens.
-- The tutor and the room are wl-scheduling resources, named by id; they are
-- copied from the batch so that one session can move to another tutor or time.
CREATE TABLE "edtech_sessions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES "edtech_batches" (id),
    number INTEGER NOT NULL,
    session_date TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    tutor_resource_id INTEGER,
    tutor_employee_id INTEGER,
    room_resource_id INTEGER,
    online_link TEXT,
    make_up BOOLEAN NOT NULL DEFAULT FALSE,
    cancel_reason TEXT,
    held_at TEXT,
    attendance_by INTEGER,
    status TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_sessions_batch_id_number" ON "edtech_sessions" (batch_id, number);
CREATE INDEX "edtech_sessions_session_date" ON "edtech_sessions" (session_date);
CREATE INDEX "edtech_sessions_status" ON "edtech_sessions" (status);
