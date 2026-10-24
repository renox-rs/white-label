-- Output reported from a line: the good quantity and the scrap of a work order.
CREATE TABLE "manufacturing_production_reports" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_order_id INTEGER NOT NULL REFERENCES "manufacturing_work_orders" (id),
    good_milli INTEGER NOT NULL DEFAULT 0,
    scrap_milli INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'posted',
    operator_id INTEGER,
    reported_at TEXT NOT NULL,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_production_reports_order" ON "manufacturing_production_reports" (work_order_id);
