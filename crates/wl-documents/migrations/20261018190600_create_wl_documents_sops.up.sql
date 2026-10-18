-- SOP library (wl-documents §3 *SOP*, #52): the SOP record, its required
-- readers and the link of a translation document to its version document.
-- Versions and translations are ordinary documents of the `sop` and
-- `sop_translation` kinds attached to the SOP as subject `documents.sop`.
CREATE TABLE "wl_documents_sops" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    title_en TEXT NOT NULL,
    title_ar TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL DEFAULT '',
    -- The legal entity; both empty (and 0) when the SOP applies to every one.
    legal_entity_type TEXT NOT NULL DEFAULT '',
    legal_entity_id INTEGER NOT NULL DEFAULT 0,
    owner_role TEXT NOT NULL,
    review_interval_months INTEGER NOT NULL DEFAULT 12,
    ack_due_days INTEGER NOT NULL DEFAULT 7,
    -- The accepted (or, once withdrawn, archived) version document.
    current_document_id INTEGER REFERENCES "wl_documents_documents" (id),
    created_at TEXT,
    updated_at TEXT
);

-- The code is unique per legal entity; an empty entity is stored as ('', 0)
-- so SQLite enforces it.
CREATE UNIQUE INDEX "wl_documents_sops_code"
    ON "wl_documents_sops" (code, legal_entity_type, legal_entity_id);

CREATE TABLE "wl_documents_sop_readers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sop_id INTEGER NOT NULL REFERENCES "wl_documents_sops" (id),
    all_staff INTEGER NOT NULL DEFAULT 0,
    -- The role or trade key; empty for an all-staff reader.
    role_key TEXT NOT NULL DEFAULT '',
    -- The scope the reader works in; both empty for every scope.
    scope_type TEXT NOT NULL DEFAULT '',
    scope_id TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_sop_readers_unique"
    ON "wl_documents_sop_readers" (sop_id, all_staff, role_key, scope_type, scope_id);

CREATE TABLE "wl_documents_sop_translations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The translation document.
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    language TEXT NOT NULL,
    -- The version document it translates.
    version_document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_documents_sop_translations_document"
    ON "wl_documents_sop_translations" (document_id);
CREATE INDEX "wl_documents_sop_translations_version"
    ON "wl_documents_sop_translations" (version_document_id, language);
