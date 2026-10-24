-- The non-conformance report (07 Manufacturing §7.1 NCR, MFG.QCL.02): a defect on a lot (or on an item and a dispatch), its cause, its disposition and what became of it. Quantities are thousandths of the base unit, money is fils.
CREATE TABLE "manufacturing_ncrs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    plant_id INTEGER NOT NULL REFERENCES "manufacturing_plants" (id),
    item_id INTEGER NOT NULL,
    lot_id INTEGER,
    lot_number TEXT,
    supplier_party_id INTEGER,
    quantity_milli INTEGER NOT NULL,
    defect TEXT NOT NULL,
    cause TEXT NOT NULL DEFAULT 'unknown',
    cause_reason TEXT,
    cause_changed_by INTEGER,
    cause_changed_at TEXT,
    origin TEXT NOT NULL DEFAULT 'incoming',
    source_type TEXT,
    source_id INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    disposition TEXT,
    outcome TEXT,
    disposition_note TEXT,
    dispositioned_by INTEGER,
    dispositioned_at TEXT,
    rework_line_id INTEGER,
    rework_bom_id INTEGER,
    rework_work_order_id INTEGER,
    return_type TEXT,
    return_id INTEGER,
    scrap_value INTEGER NOT NULL DEFAULT 0,
    action_claimed_at TEXT,
    action_done_at TEXT,
    closed_by INTEGER,
    closed_at TEXT,
    cancel_reason TEXT,
    raised_by INTEGER,
    raised_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_ncrs_number" ON "manufacturing_ncrs" (number);
CREATE INDEX "manufacturing_ncrs_status" ON "manufacturing_ncrs" (status);
CREATE INDEX "manufacturing_ncrs_lot" ON "manufacturing_ncrs" (lot_id);
CREATE INDEX "manufacturing_ncrs_supplier" ON "manufacturing_ncrs" (supplier_party_id, raised_on);
CREATE INDEX "manufacturing_ncrs_rework" ON "manufacturing_ncrs" (rework_work_order_id);
CREATE INDEX "manufacturing_ncrs_return" ON "manufacturing_ncrs" (return_id);
