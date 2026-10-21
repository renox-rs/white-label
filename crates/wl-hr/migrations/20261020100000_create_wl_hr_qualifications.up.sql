-- Qualifications (wl-hr §3 *Qualification*, HRM.QUA.01; issue #191): a
-- licence, permit or certificate of an employee, with the roles and scopes
-- (branches) it covers. The validity is the document's (wl-documents
-- `document_valid`); the dates here are for display only.
CREATE TABLE "wl_hr_qualifications" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    -- The document kind that proves it, e.g. `practitioner_licence`.
    kind TEXT NOT NULL,
    -- The type shown to staff, e.g. "Practitioner licence".
    label TEXT NOT NULL DEFAULT '',
    number TEXT NOT NULL DEFAULT '',
    issuer TEXT NOT NULL DEFAULT '',
    -- The roles it covers (JSON list of keys); empty: any role.
    roles TEXT NOT NULL DEFAULT '[]',
    -- The scopes it covers (JSON list of subjects); empty: any scope.
    scopes TEXT NOT NULL DEFAULT '[]',
    valid_from TEXT,
    valid_to TEXT,
    -- The stored document (wl-documents), when there is one.
    document_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_qualifications_employee" ON "wl_hr_qualifications" (employee_id, kind);

-- One row per qualification and expiry date that `QualificationExpired` was
-- emitted for (`HRM.RUL.10` runs once). `qualification_id` is 0 for an
-- employee document of the kind with no qualification record.
CREATE TABLE "wl_hr_qualification_expiries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    qualification_id INTEGER NOT NULL DEFAULT 0,
    expires_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (employee_id, kind, qualification_id, expires_on)
);
