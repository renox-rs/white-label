CREATE TABLE "wl_late_entry_limits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    flag_after_minutes INTEGER NOT NULL,
    approval_after_hours INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_late_entry_limits_subject" ON "wl_late_entry_limits" (subject_type);

CREATE TABLE "wl_late_entries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    record_id INTEGER,
    existing_id INTEGER,
    parent_type TEXT,
    parent_id INTEGER,
    scope_kind TEXT NOT NULL DEFAULT '',
    scope_id TEXT NOT NULL DEFAULT '',
    occurred_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    minutes_late INTEGER NOT NULL DEFAULT 0,
    needs_approval INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL,
    outage_reference TEXT,
    entered_by INTEGER NOT NULL,
    approval_request_id INTEGER REFERENCES "wl_approval_requests" (id),
    payload TEXT NOT NULL DEFAULT 'null',
    fired_runs TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'requested',
    recorded_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_late_entries_record" ON "wl_late_entries" (subject_type, record_id);
CREATE INDEX "wl_late_entries_entered" ON "wl_late_entries" (entered_at);
