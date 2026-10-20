-- Deadline tasks (wl-ticketing §3 *Task*, §4.4; base of issues #154 and
-- #155): something due by a date for the agreement it belongs to, and the
-- agreements whose tasks are held or ended.

CREATE TABLE "wl_ticketing_deadline_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- The source agreement, e.g. `accounting.engagement`.
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    party_id INTEGER,
    due_date TEXT NOT NULL,
    lead_days INTEGER NOT NULL DEFAULT 0,
    start_date TEXT NOT NULL,
    owner_user_id INTEGER,
    owner_role TEXT,
    reviewer_role TEXT,
    evidence_required INTEGER NOT NULL DEFAULT 0,
    evidence TEXT,
    needs_client_input INTEGER NOT NULL DEFAULT 0,
    recurrence TEXT NOT NULL DEFAULT 'once',
    period_end TEXT,
    -- Set while the source agreement is paused (not a state).
    held INTEGER NOT NULL DEFAULT 0,
    -- Set once it went `overdue`: a completed task is late.
    late INTEGER NOT NULL DEFAULT 0,
    -- Where the work stood when it went `overdue`.
    stage TEXT,
    started_at TEXT,
    waiting_reason TEXT,
    overdue_on TEXT,
    completed_at TEXT,
    closed_reason TEXT,
    start_notified_on TEXT,
    previous_task_id INTEGER,
    next_task_id INTEGER,
    status TEXT NOT NULL DEFAULT 'upcoming',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_deadline_tasks_status" ON "wl_ticketing_deadline_tasks" (status);
CREATE INDEX "wl_ticketing_deadline_tasks_subject"
    ON "wl_ticketing_deadline_tasks" (subject_type, subject_id);
CREATE INDEX "wl_ticketing_deadline_tasks_due_date" ON "wl_ticketing_deadline_tasks" (due_date);

-- One row per agreement whose tasks are held (`held_on`) or that ended
-- (`ended_on`).
CREATE TABLE "wl_ticketing_task_sources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    held_on TEXT,
    ended_on TEXT,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (subject_type, subject_id)
);
