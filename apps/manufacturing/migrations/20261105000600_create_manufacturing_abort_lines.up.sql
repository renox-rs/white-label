-- What an abort did with the work in progress: materials returned, materials scrapped and good output kept in quarantine.
CREATE TABLE "manufacturing_abort_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_order_id INTEGER NOT NULL REFERENCES "manufacturing_work_orders" (id),
    kind TEXT NOT NULL,
    item_id INTEGER,
    lot_id INTEGER,
    location_id INTEGER,
    quantity_milli INTEGER NOT NULL,
    value INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_abort_lines_order" ON "manufacturing_abort_lines" (work_order_id);
