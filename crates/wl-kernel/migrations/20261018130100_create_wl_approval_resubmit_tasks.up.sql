CREATE TABLE "wl_approval_resubmit_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL UNIQUE REFERENCES "wl_approval_requests" (id) ON DELETE CASCADE,
    requester_id INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    closed_reason TEXT,
    resubmission_id INTEGER REFERENCES "wl_approval_requests" (id),
    closed_by INTEGER,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_approval_resubmit_tasks_requester" ON "wl_approval_resubmit_tasks" (requester_id, status);
