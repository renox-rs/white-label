-- Expiry logs (wl-documents §5.2, #48): what the nightly run has already
-- reported, so a repeat run adds nothing.

-- A planned record reported as affected by a blocking document that expires
-- before it. Rows are kept: an item that moves and moves back is not
-- reported twice.
CREATE TABLE "wl_documents_affected_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    -- The guarded transition, `<entity>.<transition>`.
    guarded TEXT NOT NULL,
    record_type TEXT NOT NULL,
    record_id INTEGER NOT NULL,
    -- The date the record is planned for.
    planned_date TEXT NOT NULL,
    -- The business date of the run that first reported it.
    first_reported_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_affected_items_unique" ON "wl_documents_affected_items" (document_id, guarded, record_type, record_id);

-- A reminder offset (days before expiry) sent for a document.
CREATE TABLE "wl_documents_reminder_logs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    -- The offset in days before the expiry date.
    offset_days INTEGER NOT NULL,
    -- The business date of the run that sent (or passed) it.
    run_date TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_reminder_logs_unique" ON "wl_documents_reminder_logs" (document_id, offset_days);
