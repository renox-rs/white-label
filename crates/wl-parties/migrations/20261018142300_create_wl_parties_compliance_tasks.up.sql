-- The freeze and report tasks of a confirmed sanctions match (wl-parties §4,
-- `PTY.SCR.01`): one freeze task and one report task per screening, both for
-- the compliance role only (`parties.block_reason.view`).
CREATE TABLE "wl_parties_compliance_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `freeze` or `report`.
    kind TEXT NOT NULL,
    screening_id INTEGER NOT NULL REFERENCES "wl_parties_screenings" (id) ON DELETE CASCADE,
    -- `open` or `done`.
    status TEXT NOT NULL DEFAULT 'open',
    -- The flag time plus the setting's hours (UTC).
    due_at TEXT NOT NULL,
    completed_by INTEGER,
    completed_at TEXT,
    -- Freeze task: when the funds were frozen.
    frozen_at TEXT,
    -- Report task: the filing reference and the date reported.
    reference TEXT,
    reported_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

-- One task of each kind per screening; makes the creation idempotent.
CREATE UNIQUE INDEX "wl_parties_compliance_tasks_screening_kind"
    ON "wl_parties_compliance_tasks" (screening_id, kind);
CREATE INDEX "wl_parties_compliance_tasks_status" ON "wl_parties_compliance_tasks" (status);

-- A party the task names and how it stood when the match was confirmed:
-- `blocked`, `archived` or `blocked_already`.
CREATE TABLE "wl_parties_compliance_task_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES "wl_parties_compliance_tasks" (id) ON DELETE CASCADE,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    -- The restricted Block record of the screening for this party.
    block_record_id INTEGER,
    standing TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_parties_compliance_task_parties_task_party"
    ON "wl_parties_compliance_task_parties" (task_id, party_id);

-- A line of held money on a freeze task. Pre-filled unconfirmed from the
-- registered funds sources (`source_key`), or added by hand (`source_key`
-- empty, `party_id` NULL when it belongs to none of the named parties).
CREATE TABLE "wl_parties_compliance_task_funds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES "wl_parties_compliance_tasks" (id) ON DELETE CASCADE,
    party_id INTEGER,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    amount_fils INTEGER NOT NULL,
    currency TEXT NOT NULL,
    source_key TEXT NOT NULL DEFAULT '',
    confirmed INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_compliance_task_funds_task" ON "wl_parties_compliance_task_funds" (task_id);
