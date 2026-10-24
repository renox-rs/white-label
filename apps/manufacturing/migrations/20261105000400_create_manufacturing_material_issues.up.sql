-- Material issued to a work order, one row per lot, at the unit cost the issue was posted with. returned_milli is what went back at an abort. A row with a pending_issue_id is the short part waiting for stock.
CREATE TABLE "manufacturing_material_issues" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_order_id INTEGER NOT NULL REFERENCES "manufacturing_work_orders" (id),
    item_id INTEGER NOT NULL,
    lot_id INTEGER,
    location_id INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL,
    unit_cost INTEGER NOT NULL DEFAULT 0,
    returned_milli INTEGER NOT NULL DEFAULT 0,
    pending_issue_id INTEGER,
    issued_by INTEGER,
    issued_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_material_issues_order" ON "manufacturing_material_issues" (work_order_id);
