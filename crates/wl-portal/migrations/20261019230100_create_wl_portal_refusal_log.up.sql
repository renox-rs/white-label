-- The log of portal actions refused because the party the user acts for is
-- blocked (wl-portal §6, `PTY.ACL.02`, issue #44): one row per refusal. It
-- holds no reason: the portal never learns it. Only staff with
-- `parties.block_reason.view` read it.
CREATE TABLE "wl_portal_refusal_log" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    -- The refused action, e.g. `deposit_release_request`.
    action TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_portal_refusal_log_party" ON "wl_portal_refusal_log" (party_id);
