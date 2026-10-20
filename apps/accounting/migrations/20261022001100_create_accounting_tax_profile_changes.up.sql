-- 04 Accounting `ACC.ACL.13` (BL-ACC-14): a change to the calendar-driving
-- fields of a tax profile waits for a manager other than the requester; one
-- pending change per client. The contact and note columns are the fields a
-- change needs no approval for.
CREATE TABLE "accounting_tax_profile_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    base_profile_id INTEGER,
    trn TEXT,
    vat_registered BOOLEAN NOT NULL DEFAULT FALSE,
    vat_period TEXT NOT NULL DEFAULT 'not_registered',
    vat_first_period_end TEXT,
    financial_year_end TEXT,
    ct_registration_no TEXT,
    small_business_relief BOOLEAN NOT NULL DEFAULT FALSE,
    qfzp BOOLEAN NOT NULL DEFAULT FALSE,
    effective_date TEXT NOT NULL,
    changed_fields TEXT NOT NULL DEFAULT '',
    note TEXT,
    requested_by INTEGER,
    approval_request_id INTEGER,
    profile_id INTEGER,
    decided_by INTEGER,
    decided_at TEXT,
    decision_note TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_tax_profile_changes_client_party_id" ON "accounting_tax_profile_changes" (client_party_id);
CREATE INDEX "accounting_tax_profile_changes_status" ON "accounting_tax_profile_changes" (status);
CREATE UNIQUE INDEX "accounting_tax_profile_changes_one_pending" ON "accounting_tax_profile_changes" (client_party_id) WHERE status = 'pending_approval';

ALTER TABLE "accounting_tax_profiles" ADD COLUMN contact_email TEXT;
ALTER TABLE "accounting_tax_profiles" ADD COLUMN notes TEXT;
