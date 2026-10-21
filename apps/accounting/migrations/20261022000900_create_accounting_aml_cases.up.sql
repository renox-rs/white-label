-- 04 Accounting §7.1 *AML case*, `ACC.RUL.12`, `ACC.ACL.05` (BL-ACC-10): the
-- MLRO's internal case record and the confirmed matches linked to it. Visible
-- to the MLRO and deputy only (no tipping off).
CREATE TABLE "accounting_aml_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    kind TEXT NOT NULL DEFAULT 'concern',
    facts TEXT NOT NULL,
    raised_by INTEGER,
    block_note TEXT,
    closing_reason TEXT,
    closed_by INTEGER,
    closed_on TEXT,
    goaml_reference TEXT,
    goaml_date TEXT,
    reported_by INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_aml_cases_client_party_id" ON "accounting_aml_cases" (client_party_id);
CREATE INDEX "accounting_aml_cases_status" ON "accounting_aml_cases" (status);

CREATE TABLE "accounting_aml_matches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES "accounting_aml_cases" (id),
    client_party_id INTEGER NOT NULL,
    screening_id INTEGER NOT NULL,
    matched_party_id INTEGER NOT NULL,
    role TEXT NOT NULL DEFAULT 'client',
    list_name TEXT,
    list_entry TEXT,
    flagged_at TEXT,
    freeze_task_id INTEGER,
    report_task_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_aml_matches_case_id" ON "accounting_aml_matches" (case_id);
CREATE INDEX "accounting_aml_matches_screening_id" ON "accounting_aml_matches" (screening_id);
