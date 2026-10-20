-- Service contracts and what they cover (wl-ticketing §3, §4.3, §4.6, issue
-- #149): an agreement with a vendor to maintain covered subjects, the
-- maintenance plans it creates, and the contract a ticket is handled under.

CREATE TABLE "wl_ticketing_service_contracts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    vendor_party_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    renewal_notice_days INTEGER NOT NULL DEFAULT 60,
    -- Replaces the category's SLA policy for the tickets it covers.
    sla_policy_id INTEGER REFERENCES "wl_ticketing_sla_policies" (id) ON DELETE SET NULL,
    -- Yes when a covered category is compliance-critical (set at `activate`).
    compliance_contract INTEGER NOT NULL DEFAULT 0,
    previous_version_id INTEGER,
    -- The reason and date entered with `terminate`.
    termination_reason TEXT,
    termination_date TEXT,
    -- When `WRK.RUL.18` handled the end (once per contract end).
    ended_at TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_service_contracts_vendor" ON "wl_ticketing_service_contracts" (vendor_party_id);
CREATE INDEX "wl_ticketing_service_contracts_status" ON "wl_ticketing_service_contracts" (status);

-- One subject covered for one category by one contract.
CREATE TABLE "wl_ticketing_contract_coverage" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL REFERENCES "wl_ticketing_service_contracts" (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL REFERENCES "wl_ticketing_categories" (id),
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (contract_id, subject_type, subject_id, category_id)
);

CREATE INDEX "wl_ticketing_contract_coverage_subject"
    ON "wl_ticketing_contract_coverage" (subject_type, subject_id);

-- The work that recurs for a covered subject: one plan per covered subject
-- and category, created at `activate`, `ended` when the contract ends.
CREATE TABLE "wl_ticketing_maintenance_plans" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER REFERENCES "wl_ticketing_service_contracts" (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL REFERENCES "wl_ticketing_categories" (id),
    status TEXT NOT NULL DEFAULT 'active',
    ended_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_maintenance_plans_contract" ON "wl_ticketing_maintenance_plans" (contract_id);

-- The contract the ticket is handled under (a planned visit of its plan, or a
-- reactive ticket on a covered subject).
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN contract_id INTEGER;
CREATE INDEX "wl_ticketing_tickets_contract" ON "wl_ticketing_tickets" (contract_id);

-- `WRK.RUL.18` task options per category: 1 offers only "reassign" for the
-- category's open visits (01: compliance-critical categories).
ALTER TABLE "wl_ticketing_categories" ADD COLUMN contract_end_reassign_only INTEGER NOT NULL DEFAULT 0;
