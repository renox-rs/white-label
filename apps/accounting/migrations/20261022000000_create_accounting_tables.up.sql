-- 04 Accounting §6, §7.1 (base for BL-ACC-01, BL-ACC-07, BL-ACC-08): the
-- engagement, the CDD file with its screened parties, and the tax profile
-- versioned by effective date.
CREATE TABLE "accounting_engagements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    fee_basis TEXT NOT NULL DEFAULT 'retainer',
    retainer_fee INTEGER NOT NULL DEFAULT 0,
    start_date TEXT,
    manager_user_id INTEGER,
    letter_signed BOOLEAN NOT NULL DEFAULT FALSE,
    letter_signed_on TEXT,
    reason_code TEXT,
    client_messages_suppressed BOOLEAN NOT NULL DEFAULT FALSE,
    activated_on TEXT,
    ended_on TEXT,
    status TEXT NOT NULL DEFAULT 'onboarding',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_engagements_client_party_id" ON "accounting_engagements" (client_party_id);
CREATE INDEX "accounting_engagements_status" ON "accounting_engagements" (status);

CREATE TABLE "accounting_cdd_files" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    risk_rating TEXT NOT NULL DEFAULT 'unrated',
    risk_factors TEXT NOT NULL DEFAULT '',
    edd_notes TEXT NOT NULL DEFAULT '',
    identity_documents_attached BOOLEAN NOT NULL DEFAULT FALSE,
    next_review_date TEXT,
    approved_by INTEGER,
    approved_at TEXT,
    decision_reason TEXT,
    status TEXT NOT NULL DEFAULT 'in_progress',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_cdd_files_client_party_id" ON "accounting_cdd_files" (client_party_id);
CREATE INDEX "accounting_cdd_files_status" ON "accounting_cdd_files" (status);

CREATE TABLE "accounting_cdd_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cdd_file_id INTEGER NOT NULL REFERENCES "accounting_cdd_files" (id),
    party_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "accounting_cdd_parties_party" ON "accounting_cdd_parties" (cdd_file_id, party_id, role);

CREATE TABLE "accounting_tax_profiles" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    trn TEXT,
    vat_registered BOOLEAN NOT NULL DEFAULT FALSE,
    vat_period TEXT NOT NULL DEFAULT 'not_registered',
    vat_first_period_end TEXT,
    financial_year_end TEXT,
    ct_registration_no TEXT,
    small_business_relief BOOLEAN NOT NULL DEFAULT FALSE,
    qfzp BOOLEAN NOT NULL DEFAULT FALSE,
    effective_date TEXT NOT NULL,
    approved_by INTEGER,
    approved_at TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_tax_profiles_client_party_id" ON "accounting_tax_profiles" (client_party_id);
