-- The search index (wl-kernel §5.8, BL-AGT-01, decision 0006): one search
-- document per findable record of every module, and the identifiers that
-- match it exactly. The full-text index over the documents is made by the
-- next migration, from the model (`renox::db::search::migration`).
--
-- scope_type / scope_id: the record's scope as `role_user` stores it
-- ('' / '' for global). sensitivity: normal, personal, health or financial.
CREATE TABLE "wl_search_documents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    secondary TEXT NOT NULL DEFAULT '',
    keywords TEXT NOT NULL DEFAULT '',
    forms TEXT NOT NULL DEFAULT '',
    status TEXT,
    voided INTEGER NOT NULL DEFAULT 0,
    permission TEXT NOT NULL,
    portal_visible INTEGER NOT NULL DEFAULT 0,
    scope_type TEXT NOT NULL DEFAULT '',
    scope_id TEXT NOT NULL DEFAULT '',
    legal_entity_id INTEGER,
    sensitivity TEXT NOT NULL DEFAULT 'normal',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_search_documents_subject" ON "wl_search_documents" (subject_type, subject_id);
CREATE INDEX "wl_search_documents_scope" ON "wl_search_documents" (scope_type, scope_id);

-- Values matched exactly, never by words: document numbers, TRN, Emirates
-- ID, phone, unit or plate number. `value` is normalised; a sensitive one
-- (hashed = 1) is an HMAC of the normalised value under APP_KEY.
CREATE TABLE "wl_search_identifiers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    hashed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_search_identifiers_value" ON "wl_search_identifiers" (kind, value, subject_type, subject_id);
CREATE INDEX "wl_search_identifiers_subject" ON "wl_search_identifiers" (subject_type, subject_id);
