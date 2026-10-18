-- Document kinds (wl-documents §3 *Document kind*): seeded per module and
-- vertical (`KindSeed`), extended per client. A legal block is locked by the
-- platform: override `never`, compliance-critical, its entries locked.
CREATE TABLE "wl_documents_kinds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    -- JSON list of the subject types it attaches to, e.g. ["hr.employee"].
    subject_types TEXT NOT NULL DEFAULT '[]',
    has_expiry INTEGER NOT NULL DEFAULT 0,
    has_number INTEGER NOT NULL DEFAULT 0,
    -- JSON list of days before expiry to remind, e.g. [60, 30, 7].
    reminder_offsets TEXT NOT NULL DEFAULT '[]',
    sensitivity TEXT NOT NULL DEFAULT 'normal',
    retention_rule TEXT,
    -- JSON list of allowed file extensions, e.g. ["pdf", "jpg"].
    allowed_file_types TEXT NOT NULL DEFAULT '[]',
    max_size_bytes INTEGER NOT NULL DEFAULT 20971520,
    portal_upload INTEGER NOT NULL DEFAULT 0,
    needs_review INTEGER NOT NULL DEFAULT 1,
    override_mode TEXT NOT NULL DEFAULT 'never',
    legal_block INTEGER NOT NULL DEFAULT 0,
    look_ahead_days INTEGER NOT NULL DEFAULT 0,
    compliance_critical INTEGER NOT NULL DEFAULT 0,
    -- The role whose holders renew it (its responsible staff, §5.2), e.g. `pro`.
    responsible_role TEXT,
    created_at TEXT,
    updated_at TEXT
);

-- The `blocks` of a kind (§5.6): one guarded `<entity>.<transition>` each,
-- with mode `block` or `flag` and an optional condition on the guarded record.
-- A locked entry (of a legal block) cannot be removed or changed by a client.
CREATE TABLE "wl_documents_kind_blocks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind_id INTEGER NOT NULL REFERENCES "wl_documents_kinds" (id) ON DELETE CASCADE,
    guarded TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'block',
    -- The field of the guarded record the condition reads; NULL = every record.
    condition_field TEXT,
    -- JSON list of the values that make the entry apply.
    condition_values TEXT NOT NULL DEFAULT '[]',
    locked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (kind_id, guarded)
);

CREATE INDEX "wl_documents_kind_blocks_guarded" ON "wl_documents_kind_blocks" (guarded);
