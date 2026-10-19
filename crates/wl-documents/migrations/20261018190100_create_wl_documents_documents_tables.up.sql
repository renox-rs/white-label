-- Documents (wl-documents §3 *Document*, §4.1): one logical document of a
-- kind attached to a subject (`subject_type` + `subject_id`, any module's
-- record). Its status moves only through the Document machine.
CREATE TABLE "wl_documents_documents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    kind_id INTEGER NOT NULL REFERENCES "wl_documents_kinds" (id),
    title TEXT NOT NULL DEFAULT '',
    number TEXT,
    issue_date TEXT,
    expiry_date TEXT,
    issuing_authority TEXT,
    -- The number of the version in force (`wl_documents_versions.version_no`).
    current_version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'pending_review',
    -- `upload`, `generated` or `signed`.
    source TEXT NOT NULL DEFAULT 'upload',
    -- Where it came from, e.g. `back_office`, `portal`.
    channel TEXT NOT NULL DEFAULT 'back_office',
    uploaded_by INTEGER,
    submitted_at TEXT,
    reviewed_by INTEGER,
    reviewed_at TEXT,
    rejection_reason TEXT,
    accepted_at TEXT,
    superseded_by INTEGER,
    superseded_at TEXT,
    archived_at TEXT,
    -- Set when a refused guard names this pending renewal: the review queue
    -- shows it first (newest first).
    review_priority_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_documents_documents_subject_kind"
    ON "wl_documents_documents" (subject_type, subject_id, kind_id);
CREATE INDEX "wl_documents_documents_status" ON "wl_documents_documents" (status);

-- The files of a document (§3 *Document version*): a file is never
-- overwritten, a new file is a new version.
CREATE TABLE "wl_documents_versions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id) ON DELETE CASCADE,
    version_no INTEGER NOT NULL,
    -- The private storage key of the file.
    storage_key TEXT NOT NULL,
    file_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    -- SHA-256 of the file, lower-case hex.
    checksum TEXT NOT NULL,
    uploaded_by INTEGER,
    uploaded_at TEXT NOT NULL,
    note TEXT,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (document_id, version_no)
);
