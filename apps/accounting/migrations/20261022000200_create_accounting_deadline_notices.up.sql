-- 04 Accounting `ACC.RUL.09` (BL-ACC-04): the statutory deadline notice of a
-- suspension or an end, its delivery, reminder, escalation and the client
-- approver's acknowledgement in the portal.
CREATE TABLE "accounting_deadline_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    client_party_id INTEGER NOT NULL,
    cause TEXT NOT NULL,
    prepared_for TEXT,
    on_date TEXT NOT NULL,
    window_end TEXT NOT NULL,
    lines TEXT NOT NULL,
    deadline_count INTEGER NOT NULL DEFAULT 0,
    task_id INTEGER,
    sent_at TEXT,
    sent_by INTEGER,
    delivered_on TEXT,
    reminded_on TEXT,
    escalated_on TEXT,
    escalation_task_id INTEGER,
    acknowledged_at TEXT,
    acknowledged_by INTEGER,
    cancelled_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_deadline_notices_engagement_id" ON "accounting_deadline_notices" (engagement_id);
