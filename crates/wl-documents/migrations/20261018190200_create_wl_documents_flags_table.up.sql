-- Flags (wl-documents §3, #46): a record whose guarded transition ran while
-- a `flag` mode document was not valid. A flag is permanent: it is reviewed
-- (with a note) but never removed or edited otherwise.
CREATE TABLE "wl_documents_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The flagged record: the entity part of `guarded`, and its id.
    record_type TEXT NOT NULL,
    record_id INTEGER NOT NULL,
    -- The guarded transition, `<entity>.<transition>`.
    guarded TEXT NOT NULL,
    -- The document kind's key.
    kind TEXT NOT NULL,
    -- The document subject that failed, e.g. the worker.
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    -- The expired, rejected or pending document; empty when missing.
    document_id INTEGER,
    -- `expired`, `missing`, `rejected` or `pending_review`.
    reason TEXT NOT NULL,
    expires_on TEXT,
    -- The business date the document was judged on.
    on_date TEXT NOT NULL,
    label TEXT NOT NULL,
    reviewed_at TEXT,
    reviewed_by INTEGER,
    review_note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_documents_flags_record" ON "wl_documents_flags" (record_type, record_id);
CREATE INDEX "wl_documents_flags_reviewed_at" ON "wl_documents_flags" (reviewed_at);
