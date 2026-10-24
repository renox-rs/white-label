-- What MFG.RUL.09 did to work orders when a sales order was amended or
-- cancelled (BL-MFG-03): one row per change, so a replayed event does
-- nothing (the key names the event and the line) and the planner can read
-- what happened on the work order's page.
CREATE TABLE "manufacturing_order_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_key TEXT NOT NULL,
    order_id INTEGER NOT NULL,
    line_id INTEGER NOT NULL,
    work_order_id INTEGER NOT NULL DEFAULT 0,
    action TEXT NOT NULL,
    from_milli INTEGER NOT NULL DEFAULT 0,
    to_milli INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_order_adjustments_event" ON "manufacturing_order_adjustments" (event_key, action, work_order_id);
CREATE INDEX "manufacturing_order_adjustments_work_order" ON "manufacturing_order_adjustments" (work_order_id);
CREATE INDEX "manufacturing_order_adjustments_line" ON "manufacturing_order_adjustments" (order_id, line_id);
