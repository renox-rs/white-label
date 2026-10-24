-- 08 Auto fleet service contract (`AUT.FLT.01`, 08 section 4.5, section 6
-- *Fleet contract*, section 7.1 *Fleet contract*): the agreement with a
-- fleet (a party) for a term: labour rate, part price tier, inclusions,
-- notice period, early termination fee, the signed contract, and what the
-- negative flows record (suspension, termination with its notice and
-- effective dates, the day up to which monthly invoices were made). The
-- vehicles under contract are `auto_vehicles.fleet_contract_id`. The status
-- moves only through the machine.
CREATE TABLE "auto_fleet_contracts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fleet_party_id INTEGER NOT NULL,
    contact_party_id INTEGER,
    title TEXT NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    labour_rate INTEGER NOT NULL DEFAULT 0,
    price_tier TEXT,
    inclusions TEXT,
    notice_period_days INTEGER NOT NULL DEFAULT 30,
    early_termination_fee INTEGER NOT NULL DEFAULT 0,
    renewal_window_days INTEGER NOT NULL DEFAULT 60,
    signed_document TEXT,
    activated_at TEXT,
    suspended_reason TEXT,
    suspended_at TEXT,
    termination_reason TEXT,
    notice_date TEXT,
    effective_date TEXT,
    termination_scheduled BOOLEAN NOT NULL DEFAULT FALSE,
    termination_requested_by INTEGER,
    ended_at TEXT,
    invoiced_through_date TEXT,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_fleet_contracts_fleet_party_id" ON "auto_fleet_contracts" (fleet_party_id);
CREATE INDEX "auto_fleet_contracts_status" ON "auto_fleet_contracts" (status);
