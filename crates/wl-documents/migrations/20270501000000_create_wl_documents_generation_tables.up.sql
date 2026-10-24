-- Document templates (wl-documents §3 *Document template*, §5.3, §7): the
-- layout a generated document is built from. Editing a template adds a new
-- version; documents keep the version they were built from.
CREATE TABLE "wl_documents_templates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL,
    -- The document kind of the generated documents.
    kind_key TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    -- `en`, `ar` or `bilingual`.
    language_mode TEXT NOT NULL DEFAULT 'bilingual',
    body_en TEXT NOT NULL DEFAULT '',
    body_ar TEXT NOT NULL DEFAULT '',
    footer_en TEXT,
    footer_ar TEXT,
    -- `a4` or `letter`.
    paper_size TEXT NOT NULL DEFAULT 'a4',
    version INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    edited_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_templates_key_version"
    ON "wl_documents_templates" (key, version);

-- Generations (§3 *Generation*): one generated document, the template
-- version it came from, the hash of its data and the number assigned.
CREATE TABLE "wl_documents_generations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_key TEXT NOT NULL,
    template_version INTEGER NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    language_mode TEXT NOT NULL,
    data_hash TEXT NOT NULL,
    number TEXT,
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    generated_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_documents_generations_subject"
    ON "wl_documents_generations" (template_key, subject_type, subject_id);
