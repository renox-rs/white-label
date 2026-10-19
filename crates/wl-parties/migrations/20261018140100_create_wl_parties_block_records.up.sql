-- One row per `block`, and per confirmed screening match on a party already
-- blocked or archived (wl-parties §3). Holds the real reason code; restricted
-- rows are readable only under `parties.block_reason.view` (`PTY.ACL.02`).
CREATE TABLE "wl_parties_block_records" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    reason_code TEXT NOT NULL,
    restricted INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL,
    -- The user who blocked, or whose action caused a system block; NULL for a pure system run.
    blocked_by INTEGER,
    blocked_at TEXT NOT NULL,
    unblocked_by INTEGER,
    unblocked_at TEXT,
    unblock_note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_block_records_party" ON "wl_parties_block_records" (party_id);
