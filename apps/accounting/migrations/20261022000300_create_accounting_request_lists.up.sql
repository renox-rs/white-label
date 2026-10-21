-- 04 Accounting `ACC.RUL.02`, `POR.RUL.01` (BL-ACC-03, BL-ACC-05): the
-- document request list of an engagement's period, or the period skipped
-- while the engagement was suspended.
CREATE TABLE "accounting_request_lists" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    client_party_id INTEGER NOT NULL,
    period_end TEXT NOT NULL,
    skipped_on TEXT,
    requested_on TEXT,
    due_date TEXT,
    received_on TEXT,
    reminders_sent INTEGER NOT NULL DEFAULT 0,
    last_reminded_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "accounting_request_lists_engagement_id_period_end" ON "accounting_request_lists" (engagement_id, period_end);
