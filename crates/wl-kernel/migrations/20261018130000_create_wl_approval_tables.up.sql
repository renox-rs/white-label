CREATE TABLE "wl_approval_policies" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    action TEXT NOT NULL,
    condition TEXT NOT NULL DEFAULT 'always',
    threshold INTEGER NOT NULL DEFAULT 0,
    input_key TEXT,
    steps TEXT NOT NULL DEFAULT '[]',
    sod INTEGER NOT NULL DEFAULT 1,
    expiry_hours INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (action, condition, threshold)
);

CREATE TABLE "wl_approval_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    policy_id INTEGER NOT NULL REFERENCES "wl_approval_policies" (id),
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    transition TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT 'null',
    requester_id INTEGER NOT NULL,
    current_step INTEGER NOT NULL DEFAULT 1,
    steps TEXT NOT NULL DEFAULT '[]',
    sod INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'pending',
    expires_at TEXT,
    subject_state_before TEXT NOT NULL,
    subject_moved_to TEXT,
    state_returned_to TEXT,
    released_at TEXT,
    resubmission_of INTEGER REFERENCES "wl_approval_requests" (id),
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_approval_requests_subject" ON "wl_approval_requests" (subject_type, subject_id, status);

CREATE TABLE "wl_approval_decisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL REFERENCES "wl_approval_requests" (id) ON DELETE CASCADE,
    step INTEGER NOT NULL,
    approver_id INTEGER NOT NULL,
    decision TEXT NOT NULL,
    comment TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_approval_decisions_request" ON "wl_approval_decisions" (request_id);
