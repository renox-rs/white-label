-- Expiry and renewal (wl-documents §5.2, #49): what the nightly run has
-- already reported as expired, and the renewal tasks it opened.

-- Roles told as well as the responsible role (JSON list), e.g. ["hr"].
ALTER TABLE "wl_documents_kinds" ADD COLUMN also_notify TEXT NOT NULL DEFAULT '[]';

-- A document whose expiry the run has reported. Unique per document: a missed
-- night is caught up, and the event is never emitted twice.
CREATE TABLE "wl_documents_expired_logs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    -- The business date of the run that reported it.
    run_date TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_expired_logs_unique" ON "wl_documents_expired_logs" (document_id);

-- A renewal task opened for an expired document. The row stays when the task
-- is closed (`closed_at`), so the history is kept.
CREATE TABLE "wl_documents_renewal_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    kind_id INTEGER NOT NULL REFERENCES "wl_documents_kinds" (id),
    -- The expired document that caused the task.
    expired_document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    -- The ticket; NULL when ticketing is off.
    ticket_id INTEGER,
    ticket_number TEXT,
    opened_at TEXT NOT NULL,
    closed_at TEXT,
    -- The accepted document that closed it.
    renewal_document_id INTEGER REFERENCES "wl_documents_documents" (id),
    created_at TEXT,
    updated_at TEXT
);

-- At most one open task per subject and kind.
CREATE UNIQUE INDEX "wl_documents_renewal_tasks_one_open" ON "wl_documents_renewal_tasks" (subject_type, subject_id, kind_id) WHERE closed_at IS NULL;
