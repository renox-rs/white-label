-- Compliance flags (wl-ticketing §3, issue #148): a covered subject that is
-- not compliant for a compliance-critical category.

-- Set by the vertical (01: fire and life safety, lifts).
ALTER TABLE "wl_ticketing_categories" ADD COLUMN compliance_critical INTEGER NOT NULL DEFAULT 0;

-- A planned visit: the due date of the maintenance plan occurrence that
-- opened it (`MaintenanceDue`); not the visit scheduled at, not an SLA time.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN planned_due_date TEXT;
-- When `WRK.RUL.14` told the coordinator the visit was missed: once.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN missed_notified_at TEXT;

CREATE TABLE "wl_ticketing_compliance_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL REFERENCES "wl_ticketing_categories" (id),
    -- `no_contract`, `visit_missed` or `vendor_document_expired`.
    reason TEXT NOT NULL,
    -- The service contract, when the flag is about one.
    contract_id INTEGER,
    -- The visit ticket of a `visit_missed` flag.
    ticket_id INTEGER,
    raised_at TEXT NOT NULL,
    cleared_at TEXT,
    -- Why it cleared: `visit resolved`, `replacement contract active`,
    -- `document renewed`.
    cleared_by TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_compliance_flags_subject"
    ON "wl_ticketing_compliance_flags" (subject_type, subject_id);

-- One open flag per subject, category and reason.
CREATE UNIQUE INDEX "wl_ticketing_compliance_flags_one_open"
    ON "wl_ticketing_compliance_flags" (subject_type, subject_id, category_id, reason)
    WHERE cleared_at IS NULL;
