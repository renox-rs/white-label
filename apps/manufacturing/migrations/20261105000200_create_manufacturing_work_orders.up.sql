-- The work order (07 Manufacturing §6, §7.1): an instruction to produce a quantity of one item on one line. Quantities are thousandths of the base unit, money is fils.
CREATE TABLE "manufacturing_work_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    plant_id INTEGER NOT NULL REFERENCES "manufacturing_plants" (id),
    line_id INTEGER REFERENCES "manufacturing_lines" (id),
    item_id INTEGER NOT NULL,
    bom_id INTEGER,
    quantity_milli INTEGER NOT NULL,
    quantity_for_stock_milli INTEGER NOT NULL DEFAULT 0,
    planned_start TEXT,
    planned_end TEXT,
    source TEXT NOT NULL DEFAULT 'stock',
    sales_order_id INTEGER,
    sales_order_line_id INTEGER,
    status TEXT NOT NULL DEFAULT 'planned',
    hold_reason TEXT,
    std_material_cost INTEGER NOT NULL DEFAULT 0,
    std_unit_cost INTEGER NOT NULL DEFAULT 0,
    started_at TEXT,
    stopped_at TEXT,
    output_lot_id INTEGER,
    good_milli INTEGER NOT NULL DEFAULT 0,
    scrap_milli INTEGER NOT NULL DEFAULT 0,
    has_issue INTEGER NOT NULL DEFAULT 0,
    actual_cost INTEGER,
    cancel_reason TEXT,
    unlink_reason TEXT,
    unlinked_at TEXT,
    abort_reason TEXT,
    abort_requested_by INTEGER,
    abort_approved_by INTEGER,
    abort_scrap_value INTEGER,
    abort_loss INTEGER,
    aborted_at TEXT,
    abort_announced_at TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_work_orders_number" ON "manufacturing_work_orders" (number);
CREATE INDEX "manufacturing_work_orders_status" ON "manufacturing_work_orders" (status);
CREATE INDEX "manufacturing_work_orders_line" ON "manufacturing_work_orders" (line_id);
CREATE INDEX "manufacturing_work_orders_order" ON "manufacturing_work_orders" (sales_order_id, sales_order_line_id);
