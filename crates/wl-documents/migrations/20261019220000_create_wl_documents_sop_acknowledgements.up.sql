-- SOP acknowledgements (wl-documents §3 *Acknowledgement*, §4.3, issue #53):
-- one reader's confirmation that they read one SOP version. The status moves
-- only through the Acknowledgement machine. An acknowledgement holds personal
-- data (IP address, device, drawn signature): it is kept as long as wl-hr
-- keeps the worker file (§10).
CREATE TABLE "wl_documents_sop_acknowledgements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sop_id INTEGER NOT NULL REFERENCES "wl_documents_sops" (id),
    -- The SOP version document this asks for.
    document_id INTEGER NOT NULL REFERENCES "wl_documents_documents" (id),
    -- The running number of that version (1, 2, ...).
    version_no INTEGER NOT NULL,
    -- The reader: an `hr.employee`.
    employee_id INTEGER NOT NULL,
    requested_on TEXT NOT NULL,
    due_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    -- What the reader opened in their session: the version or a translation,
    -- and its checksum then.
    opened_document_id INTEGER,
    opened_checksum TEXT,
    opened_at TEXT,
    -- Evidence of `acknowledge`.
    acknowledged_at TEXT,
    acknowledged_by INTEGER,
    ack_document_id INTEGER,
    language TEXT NOT NULL DEFAULT '',
    checksum TEXT NOT NULL DEFAULT '',
    -- `click-to-accept` in the reader's session, `drawn` in person.
    method TEXT NOT NULL DEFAULT '',
    ip_address TEXT,
    user_agent TEXT,
    signature TEXT,
    witness_id INTEGER,
    -- When the reader was told, on their own channel, of an in-person record.
    in_person_notified_at TEXT,
    reminders_sent INTEGER NOT NULL DEFAULT 0,
    last_reminded_on TEXT,
    escalated_on TEXT,
    -- Why it was replaced or cancelled.
    ended_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_documents_sop_acks_sop" ON "wl_documents_sop_acknowledgements" (sop_id, document_id, status);
CREATE INDEX "wl_documents_sop_acks_employee" ON "wl_documents_sop_acknowledgements" (employee_id, status);
