-- One run of the give-up rule `NTF.RUL.01` for one message (#55 A9).
CREATE TABLE "wl_notify_reaction_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The rule id, `NTF.RUL.01`.
    rule TEXT NOT NULL,
    -- The occurrence key: the message that gave up.
    message_id INTEGER NOT NULL,
    -- The queue job that runs it; its retries continue this run.
    job_id INTEGER,
    -- `running`, `succeeded`, `skipped` or `failed`.
    status TEXT NOT NULL DEFAULT 'running',
    attempts INTEGER NOT NULL DEFAULT 0,
    -- The last error.
    error TEXT,
    -- JSON list of the reactions, in run order.
    reactions TEXT NOT NULL DEFAULT '[]',
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

-- One run per message; a run started again for it is stored as `skipped`.
CREATE UNIQUE INDEX "wl_notify_reaction_runs_occurrence"
    ON "wl_notify_reaction_runs" (rule, message_id)
    WHERE status <> 'skipped';
CREATE INDEX "wl_notify_reaction_runs_message" ON "wl_notify_reaction_runs" (message_id);

-- One reaction a run applied.
CREATE TABLE "wl_notify_reaction_steps" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL,
    -- `staff_task`, `alternative_recipient`, `mark_contact_invalid` or `statutory_notice`.
    reaction TEXT NOT NULL DEFAULT 'none',
    -- 1-based place in the run order.
    position INTEGER NOT NULL DEFAULT 0,
    -- What the handler did, e.g. `recorded`.
    status TEXT NOT NULL DEFAULT 'recorded',
    -- JSON details from the handler.
    detail TEXT NOT NULL DEFAULT '{}',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_notify_reaction_steps_reaction"
    ON "wl_notify_reaction_steps" (run_id, reaction);
