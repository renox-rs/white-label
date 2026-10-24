-- MFG.RUL.10 (BL-MFG-07): the inspection of each returned lot in quarantine,
-- and the sales decision per order line (deliver again or cancel the open
-- quantity), with what was delivered again or cancelled.
CREATE TABLE "manufacturing_return_inspections" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_key TEXT NOT NULL,
    dispatch_id INTEGER NOT NULL,
    dispatch_number TEXT NOT NULL DEFAULT '',
    order_id INTEGER NOT NULL,
    line_id INTEGER,
    item_id INTEGER NOT NULL,
    lot_id INTEGER,
    lot_key INTEGER NOT NULL DEFAULT 0,
    quantity_milli INTEGER NOT NULL,
    reason TEXT,
    returned INTEGER NOT NULL DEFAULT 0,
    run_id INTEGER,
    complaint_id INTEGER,
    task_id INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    ncr_id INTEGER,
    note TEXT,
    decided_by INTEGER,
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_return_inspections_event" ON "manufacturing_return_inspections" (event_key, item_id, lot_key);
CREATE INDEX "manufacturing_return_inspections_lot" ON "manufacturing_return_inspections" (lot_id);
CREATE INDEX "manufacturing_return_inspections_status" ON "manufacturing_return_inspections" (status);

CREATE TABLE "manufacturing_return_decisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dispatch_id INTEGER NOT NULL,
    dispatch_number TEXT NOT NULL DEFAULT '',
    order_id INTEGER NOT NULL,
    order_number TEXT NOT NULL DEFAULT '',
    line_id INTEGER NOT NULL,
    item_id INTEGER NOT NULL,
    customer_id INTEGER,
    owner_id INTEGER,
    refused_milli INTEGER NOT NULL,
    open_milli INTEGER NOT NULL,
    reason TEXT,
    task_id INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    closed_why TEXT,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_return_decisions_line" ON "manufacturing_return_decisions" (dispatch_id, line_id);
CREATE INDEX "manufacturing_return_decisions_order" ON "manufacturing_return_decisions" (order_id, line_id);
CREATE INDEX "manufacturing_return_decisions_status" ON "manufacturing_return_decisions" (status);

CREATE TABLE "manufacturing_return_moves" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_id INTEGER NOT NULL,
    move_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    quantity_milli INTEGER NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_return_moves_key" ON "manufacturing_return_moves" (decision_id, move_key);
