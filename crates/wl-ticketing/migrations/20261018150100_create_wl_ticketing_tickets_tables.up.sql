-- Tickets and their assignments (wl-ticketing §3, §4.1). A ticket is about a
-- subject (any record), has one status that only a transition changes, and at
-- most one assignee: a staff user or a vendor party. The SLA columns hold the
-- due times the first assignment sets (a reassignment never recomputes them)
-- and the clock the hold and reopen transitions move.

CREATE TABLE "wl_ticketing_tickets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    source_type TEXT,
    source_id INTEGER,
    category_id INTEGER NOT NULL REFERENCES "wl_ticketing_categories" (id),
    priority TEXT NOT NULL DEFAULT 'normal',
    requester_user_id INTEGER,
    requester_party_id INTEGER,
    channel TEXT NOT NULL DEFAULT 'system',
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    assignee_user_id INTEGER,
    assignee_party_id INTEGER,
    visit_at TEXT,
    sla_policy_id INTEGER REFERENCES "wl_ticketing_sla_policies" (id) ON DELETE SET NULL,
    sla_started_at TEXT,
    response_due TEXT,
    resolution_due TEXT,
    responded_at TEXT,
    sla_paused_at TEXT,
    -- Working minutes the resolution clock stood still (holds, time resolved).
    sla_paused_minutes INTEGER NOT NULL DEFAULT 0,
    hold_reason_id INTEGER REFERENCES "wl_ticketing_hold_reasons" (id) ON DELETE SET NULL,
    resolved_at TEXT,
    cancel_reason TEXT,
    confidential INTEGER NOT NULL DEFAULT 0,
    chargeable INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT,
    CHECK (requester_user_id IS NULL OR requester_party_id IS NULL),
    CHECK (assignee_user_id IS NULL OR assignee_party_id IS NULL)
);

CREATE INDEX "wl_ticketing_tickets_status" ON "wl_ticketing_tickets" (status);
CREATE INDEX "wl_ticketing_tickets_subject" ON "wl_ticketing_tickets" (subject_type, subject_id);
CREATE INDEX "wl_ticketing_tickets_assignee_user" ON "wl_ticketing_tickets" (assignee_user_id);

-- The reassignment history: one row per `assign` (first assignment,
-- reassignment, or a changed visit time), written in the transition.
CREATE TABLE "wl_ticketing_assignments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES "wl_ticketing_tickets" (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    previous_assignee_user_id INTEGER,
    previous_assignee_party_id INTEGER,
    assignee_user_id INTEGER,
    assignee_party_id INTEGER,
    reason_code TEXT,
    reason_text TEXT,
    visit_at TEXT,
    by_user_id INTEGER,
    at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_assignments_ticket" ON "wl_ticketing_assignments" (ticket_id);
