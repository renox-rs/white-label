-- A payment recovered, in part or in full, on a written-off invoice (#66).
CREATE TABLE "wl_invoicing_recoveries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    -- The money's source, e.g. `payments.payment:7`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The allocation the recovery wrote.
    allocation_id INTEGER NOT NULL,
    -- The amount recovered, and the total recovered after it, in fils.
    amount INTEGER NOT NULL,
    recovered_total INTEGER NOT NULL,
    -- The VAT share of the amount, in AED fils (0 without relief).
    vat_share INTEGER NOT NULL DEFAULT 0,
    -- Whether the relief was claimed (included) when it was recovered.
    relief_claimed INTEGER NOT NULL DEFAULT 0,
    -- The status of the bad-debt relief it saw; NULL without a relief.
    relief_status TEXT,
    -- The relief adjustment, and the recovery adjustment it created.
    relief_adjustment_id INTEGER,
    recovery_adjustment_id INTEGER,
    -- The day it was recovered, Asia/Dubai.
    recovered_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_recoveries_invoice" ON "wl_invoicing_recoveries" (invoice_id);
CREATE INDEX "wl_invoicing_recoveries_source" ON "wl_invoicing_recoveries" (source_type, source_id);
-- One recovery per allocation.
CREATE UNIQUE INDEX "wl_invoicing_recoveries_allocation"
    ON "wl_invoicing_recoveries" (allocation_id);
