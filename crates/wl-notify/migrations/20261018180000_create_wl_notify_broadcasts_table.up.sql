-- One marketing broadcast to a segment (#61 A3). Its status changes only
-- through the broadcast machine.
CREATE TABLE "wl_notify_broadcasts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    type_key TEXT NOT NULL,
    -- The requester (a campaign), as a `Subject`.
    requester_type TEXT NOT NULL,
    requester_id INTEGER NOT NULL,
    requested_by INTEGER NOT NULL,
    channel TEXT NOT NULL,
    segment_id INTEGER NOT NULL,
    send_at TEXT NOT NULL,
    -- The recipients who would pass the checks when it was asked.
    estimate INTEGER NOT NULL DEFAULT 0,
    -- The recipients resolved when it started.
    recipients_resolved INTEGER,
    -- JSON: members left out, counted by reason. Never names.
    excluded TEXT NOT NULL DEFAULT '{}',
    -- JSON: sent, delivered, failed, opted out.
    results TEXT NOT NULL DEFAULT '{}',
    approved_by INTEGER,
    approval_request_id INTEGER,
    cancel_reason TEXT,
    started_at TEXT,
    completed_at TEXT,
    lock_version INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_notify_broadcasts_status" ON "wl_notify_broadcasts" (status, send_at);
CREATE INDEX "wl_notify_broadcasts_requester"
    ON "wl_notify_broadcasts" (requester_type, requester_id);
