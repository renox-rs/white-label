-- 08 Auto consolidated fleet invoice (`FIN.USG.01`, `AUT.RUL.05`): one period
-- of a contract billed in one invoice with a line per vehicle, job and PO
-- reference. A period with nothing to bill is recorded too (no invoice), so
-- that the contract knows up to which day it was invoiced.
CREATE TABLE "auto_fleet_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL REFERENCES "auto_fleet_contracts" (id),
    period_start_date TEXT NOT NULL,
    period_end_date TEXT NOT NULL,
    kind TEXT NOT NULL,
    invoice_id INTEGER,
    jobs INTEGER NOT NULL DEFAULT 0,
    net_total INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_fleet_invoices_contract_id_period_start_date" ON "auto_fleet_invoices" (contract_id, period_start_date);
