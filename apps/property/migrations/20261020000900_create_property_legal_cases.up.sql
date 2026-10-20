-- The legal case flag of a lease (01 Property §7.1 *Lease* "Legal case flag",
-- BL-PRP-13): one row per flag; a lease may carry several, each cleared
-- separately with an outcome. Every flag has its ticket of the category
-- *legal case*.
CREATE TABLE "property_legal_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The id of version 1: the lease all versions belong to.
    root_id INTEGER NOT NULL,
    -- The version the flag was set on.
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id),
    -- arrears, eviction, deposit_dispute or other.
    case_type TEXT NOT NULL DEFAULT 'arrears',
    -- The Rental Dispute Centre or court reference; empty while prepared.
    reference TEXT,
    -- open or cleared.
    status TEXT NOT NULL DEFAULT 'open',
    -- paid, settled, judgment or eviction_executed, once cleared.
    outcome TEXT,
    note TEXT,
    ticket_id INTEGER,
    ticket_number TEXT,
    -- The cheque that was referred to legal, when the flag came from it.
    cheque_id INTEGER,
    opened_by INTEGER,
    opened_on TEXT NOT NULL,
    cleared_by INTEGER,
    cleared_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_legal_cases_root" ON "property_legal_cases" (root_id, status);
