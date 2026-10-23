-- 08 Auto fleet job (`AUT.FLT.01`, `FIN.USG.01`): a job card under a fleet
-- contract, with the fleet's PO reference. It is billed in the contract's
-- consolidated monthly invoice, not on delivery (`invoice_id`).
CREATE TABLE "auto_fleet_jobs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL REFERENCES "auto_fleet_contracts" (id),
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    vehicle_id INTEGER NOT NULL,
    po_reference TEXT,
    invoice_id INTEGER,
    invoiced_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_fleet_jobs_job_card_id" ON "auto_fleet_jobs" (job_card_id);
CREATE INDEX "auto_fleet_jobs_contract_id" ON "auto_fleet_jobs" (contract_id);
