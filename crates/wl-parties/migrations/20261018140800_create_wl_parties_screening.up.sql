-- Sanctions screenings (wl-parties §4, `PTY.SCR.01`): one row per run, its
-- match candidates, and the compliance-only review task a possible match opens.
CREATE TABLE "wl_parties_screening_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    -- `open` or `closed`.
    status TEXT NOT NULL DEFAULT 'open',
    -- Flag time plus 24 hours, UTC.
    due_at TEXT NOT NULL,
    flagged_at TEXT NOT NULL,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_screening_tasks_party" ON "wl_parties_screening_tasks" (party_id);
CREATE INDEX "wl_parties_screening_tasks_status" ON "wl_parties_screening_tasks" (status);

CREATE TABLE "wl_parties_screenings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    -- Why it ran: `onboarding`, `list_update`, `review` or `manual`.
    run_trigger TEXT NOT NULL,
    -- `automatic` or `manual`.
    method TEXT NOT NULL DEFAULT 'automatic',
    -- `pending`, `clear`, `possible_match`, `false_positive` or `confirmed_match`.
    status TEXT NOT NULL DEFAULT 'pending',
    run_at TEXT NOT NULL,
    -- The reviewer who recorded a manual run.
    reviewer_user INTEGER,
    -- Whether a list used was stale at the run.
    lists_stale INTEGER NOT NULL DEFAULT 0,
    review_note TEXT,
    -- The compliance task a possible match is listed on.
    task_id INTEGER REFERENCES "wl_parties_screening_tasks" (id) ON DELETE SET NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_screenings_party" ON "wl_parties_screenings" (party_id);
CREATE INDEX "wl_parties_screenings_status" ON "wl_parties_screenings" (status);

CREATE TABLE "wl_parties_screening_candidates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    screening_id INTEGER NOT NULL REFERENCES "wl_parties_screenings" (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    list_name TEXT NOT NULL,
    score REAL NOT NULL,
    -- The entry on the list; required for a manual flag.
    list_entry TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_screening_candidates_screening" ON "wl_parties_screening_candidates" (screening_id);
