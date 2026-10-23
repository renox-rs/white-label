-- 10 EdTech tutor assignment (`EDU.ACL.14`, 10 section 7.1 *Batch*
-- `change_tutor`, *Session* `change_tutor`): who teaches a batch, or one of
-- its sessions, and from when to when. The tutor is a wl-scheduling resource
-- with the wl-hr employee behind it. A row with a session is that session's
-- own tutor; a row without one is the batch's.
CREATE TABLE "edtech_tutor_assignments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES "edtech_batches" (id),
    session_id INTEGER REFERENCES "edtech_sessions" (id),
    tutor_resource_id INTEGER NOT NULL,
    tutor_employee_id INTEGER,
    valid_from TEXT NOT NULL,
    valid_to TEXT,
    assigned_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "edtech_tutor_assignments_batch_id" ON "edtech_tutor_assignments" (batch_id);
CREATE INDEX "edtech_tutor_assignments_session_id" ON "edtech_tutor_assignments" (session_id);
CREATE INDEX "edtech_tutor_assignments_tutor_resource_id" ON "edtech_tutor_assignments" (tutor_resource_id);
