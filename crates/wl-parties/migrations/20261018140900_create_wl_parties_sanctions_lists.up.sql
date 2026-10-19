-- Sanctions lists (wl-parties §3, `PTY.RUL.02`, issue #40): one row per list
-- with its last successful update, version and freshness.
CREATE TABLE "wl_parties_sanctions_lists" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    -- Where the list comes from, e.g. the publisher.
    source TEXT NOT NULL,
    -- The version last downloaded; none before the first download.
    version TEXT,
    -- When a download last succeeded, UTC; none before the first one.
    last_updated_at TEXT,
    -- `download` or `manual`.
    update_source TEXT,
    -- `current` or `stale`.
    status TEXT NOT NULL DEFAULT 'stale',
    -- When the list turned stale; cleared when it is current again.
    stale_since TEXT,
    -- The last download attempt, successful or not.
    last_attempt_at TEXT,
    -- Why the last attempt failed; cleared by a success.
    last_error TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_sanctions_lists_status" ON "wl_parties_sanctions_lists" (status);
