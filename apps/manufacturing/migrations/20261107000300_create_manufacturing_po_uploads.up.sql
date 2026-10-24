-- The customer POs a buyer uploaded in the portal (BL-MFG-06): one row per
-- customer, PO number and call-off number; the unique key stops a concurrent
-- duplicate.
CREATE TABLE "manufacturing_po_uploads" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    po TEXT NOT NULL,
    call_off TEXT NOT NULL DEFAULT '',
    po_key TEXT NOT NULL,
    call_off_key TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    uploaded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_po_uploads_key" ON "manufacturing_po_uploads" (party_id, po_key, call_off_key);
