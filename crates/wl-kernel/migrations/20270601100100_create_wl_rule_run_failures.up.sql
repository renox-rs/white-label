-- One row each time a rule run reaches `failed` or `stopped` (PLT.RUL.03):
-- a rerun clears the run's own times, so the digest reads these.
CREATE TABLE "wl_rule_run_failures" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL,
    rule TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    occurrence_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    error TEXT,
    at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_rule_run_failures_at" ON "wl_rule_run_failures" (at);
CREATE INDEX "wl_rule_run_failures_run" ON "wl_rule_run_failures" (run_id);
