-- The dispositions an NCR had, one row each: a reopened NCR keeps the earlier one here (07 Manufacturing §7.1 NCR reopen).
CREATE TABLE "manufacturing_ncr_decisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ncr_id INTEGER NOT NULL REFERENCES "manufacturing_ncrs" (id),
    disposition TEXT NOT NULL,
    outcome TEXT,
    quantity_milli INTEGER NOT NULL,
    decided_by INTEGER,
    decided_at TEXT NOT NULL,
    ended_at TEXT,
    ended_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_ncr_decisions_ncr" ON "manufacturing_ncr_decisions" (ncr_id);
