-- The quality disputes an NCR is linked to (BL-MFG-13): one row per dispute and NCR.
CREATE TABLE "manufacturing_ncr_links" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ncr_id INTEGER NOT NULL REFERENCES "manufacturing_ncrs" (id),
    dispute_id INTEGER NOT NULL,
    invoice_id INTEGER NOT NULL,
    ticket_id INTEGER,
    text TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_ncr_links_pair" ON "manufacturing_ncr_links" (dispute_id, ncr_id);
CREATE INDEX "manufacturing_ncr_links_ncr" ON "manufacturing_ncr_links" (ncr_id);
