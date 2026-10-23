-- The material pick list of a work order, generated at release and withdrawn when the work order is cancelled.
CREATE TABLE "manufacturing_pick_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_order_id INTEGER NOT NULL REFERENCES "manufacturing_work_orders" (id),
    item_id INTEGER NOT NULL,
    required_milli INTEGER NOT NULL,
    withdrawn INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_pick_lines_order" ON "manufacturing_pick_lines" (work_order_id);
