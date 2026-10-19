CREATE TABLE "wl_compliance_rules" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule TEXT NOT NULL,
    owner_role TEXT,
    critical INTEGER NOT NULL DEFAULT 1,
    watched_since TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_compliance_rules_rule" ON "wl_compliance_rules" (rule);

CREATE TABLE "wl_compliance_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    occurrence_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    error TEXT,
    planned_at TEXT,
    owner_role TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    closed_how TEXT,
    closed_by INTEGER,
    note TEXT,
    attachment_document_id INTEGER,
    closed_at TEXT,
    ticket_id INTEGER,
    ticket_number TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_compliance_alerts_occurrence" ON "wl_compliance_alerts" (rule, subject_type, subject_id, occurrence_key);
CREATE INDEX "wl_compliance_alerts_status" ON "wl_compliance_alerts" (status);
