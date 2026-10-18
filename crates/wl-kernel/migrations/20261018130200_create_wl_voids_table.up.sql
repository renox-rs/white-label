CREATE TABLE "wl_voids" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    number TEXT,
    series_key TEXT,
    state_before TEXT NOT NULL,
    reason TEXT NOT NULL,
    voided_by INTEGER NOT NULL,
    approval_request_id INTEGER REFERENCES "wl_approval_requests" (id),
    effects TEXT NOT NULL DEFAULT '[]',
    at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_voids_subject" ON "wl_voids" (subject_type, subject_id);
CREATE INDEX "wl_voids_series" ON "wl_voids" (series_key);
