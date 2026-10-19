CREATE TABLE "wl_rule_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    occurrence_key TEXT NOT NULL,
    chain_depth INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    next_attempt_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_rule_runs_occurrence" ON "wl_rule_runs" (rule, subject_type, subject_id, occurrence_key);
CREATE INDEX "wl_rule_runs_status" ON "wl_rule_runs" (status);
