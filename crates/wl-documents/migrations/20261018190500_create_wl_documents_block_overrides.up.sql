-- Block overrides (wl-documents §3 *Block override*, #50): the one-use
-- permission to run a guarded transition although a `with_approval` kind
-- blocks it. The status moves only through the BlockOverride machine; the
-- machine history is the audit (both users, the reason, the time of each step).
CREATE TABLE "wl_documents_block_overrides" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind_id INTEGER NOT NULL REFERENCES "wl_documents_kinds" (id),
    -- The document subject the kind is required for.
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    -- The guarded transition, `<entity>.<transition>`, and its record.
    guarded TEXT NOT NULL,
    record_id INTEGER NOT NULL,
    -- The record's scope when requested; both empty for global. Kept so a
    -- later move of the record does not change who may approve.
    scope_type TEXT,
    scope_id TEXT,
    reason TEXT NOT NULL,
    requested_by INTEGER NOT NULL,
    approved_by INTEGER,
    approved_at TEXT,
    valid_until TEXT,
    used_at TEXT,
    approval_request_id INTEGER,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_documents_block_overrides_lookup"
    ON "wl_documents_block_overrides" (kind_id, subject_type, subject_id, guarded, record_id);
CREATE INDEX "wl_documents_block_overrides_requested_by"
    ON "wl_documents_block_overrides" (requested_by, status);
