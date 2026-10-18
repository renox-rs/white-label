-- The statement behind an unblock (wl-parties §4 `unblock`): for a
-- `sanctions` block, `delisted` or `false_match`; NULL otherwise.
ALTER TABLE "wl_parties_block_records" ADD COLUMN unblock_basis TEXT;

-- Evidence files attached to a Block record when it is closed. The files
-- are private, on the default storage disk; `storage_key` locates one.
CREATE TABLE "wl_parties_unblock_evidence" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    block_record_id INTEGER NOT NULL REFERENCES "wl_parties_block_records" (id) ON DELETE CASCADE,
    storage_key TEXT NOT NULL,
    file_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    uploaded_by INTEGER,
    uploaded_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_unblock_evidence_record" ON "wl_parties_unblock_evidence" (block_record_id);
