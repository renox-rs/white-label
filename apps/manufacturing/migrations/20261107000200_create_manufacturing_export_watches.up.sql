-- The export dispatches MFG.RUL.08 follows (BL-MFG-12): the day each step
-- fired, so a run twice on a day changes nothing.
CREATE TABLE "manufacturing_export_watches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dispatch_id INTEGER NOT NULL,
    number TEXT NOT NULL,
    customer_id INTEGER,
    dispatched_on TEXT NOT NULL,
    warned_on TEXT,
    flagged_on TEXT,
    evidence_on TEXT,
    late_told INTEGER NOT NULL DEFAULT 0,
    task_id INTEGER,
    invoices TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_export_watches_dispatch" ON "manufacturing_export_watches" (dispatch_id);

-- The customer's note on the VAT when the zero rate is lost.
CREATE TABLE "manufacturing_contract_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    set_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_contract_notes_party" ON "manufacturing_contract_notes" (party_id);
