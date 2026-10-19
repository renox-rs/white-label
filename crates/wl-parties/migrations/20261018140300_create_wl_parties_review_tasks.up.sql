-- Review tasks for a blocked party (wl-parties §5.6): one per (party,
-- agreement kind, scope) when the party is blocked and the kind's block
-- effect is `review`. Holds only the display reason, never a restricted code.
CREATE TABLE "wl_parties_review_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    -- The agreement kind key, e.g. `booking`.
    agreement_kind TEXT NOT NULL,
    -- Where the agreements live; both NULL when global.
    scope_kind TEXT,
    scope_id INTEGER,
    -- The role that decides; NULL when no decider is configured.
    decider_role TEXT,
    -- The reason code others may see: the code, or `on_hold`.
    display_reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    -- The user who took the task; NULL puts it back to the decider role.
    assigned_user INTEGER,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_review_tasks_party" ON "wl_parties_review_tasks" (party_id);
CREATE INDEX "wl_parties_review_tasks_status" ON "wl_parties_review_tasks" (status);

-- One open agreement of the task and the decision on it.
CREATE TABLE "wl_parties_review_task_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES "wl_parties_review_tasks" (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    label TEXT NOT NULL,
    -- `pending`, `pause`, `continue` or `end`.
    decision TEXT NOT NULL DEFAULT 'pending',
    decided_by INTEGER,
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_review_task_items_task" ON "wl_parties_review_task_items" (task_id);

-- A note on the task; `by_user` is NULL for the system.
CREATE TABLE "wl_parties_review_task_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES "wl_parties_review_tasks" (id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    by_user INTEGER,
    noted_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_review_task_notes_task" ON "wl_parties_review_task_notes" (task_id);
