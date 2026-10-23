-- The Dispatch and its lines (wl-inventory §3, §4 *Dispatch*; base for
-- BL-STK-26, #245), and what the cancellation of a sales order line did to
-- them. The status moves only through the machine.
CREATE TABLE "wl_inventory_dispatches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    scope_type TEXT,
    scope_id INTEGER,
    -- The record it fulfils, e.g. `sales.sales_order:7`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    recipient_party_id INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    cancel_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_dispatches_status" ON "wl_inventory_dispatches" (status);
CREATE INDEX "wl_inventory_dispatches_source" ON "wl_inventory_dispatches" (source_type, source_id);

CREATE TABLE "wl_inventory_dispatch_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dispatch_id INTEGER NOT NULL REFERENCES "wl_inventory_dispatches" (id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- Thousandths of the base unit.
    quantity_milli INTEGER NOT NULL,
    -- The sale line (in wl-sales) it fulfils.
    sale_line_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_dispatch_lines_dispatch" ON "wl_inventory_dispatch_lines" (dispatch_id);
CREATE INDEX "wl_inventory_dispatch_lines_sale_line" ON "wl_inventory_dispatch_lines" (sale_line_id);

CREATE TABLE "wl_inventory_cancel_effects" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Once per event and line.
    effect_key TEXT NOT NULL,
    order_id INTEGER NOT NULL,
    sale_line_id INTEGER NOT NULL,
    outcome TEXT,
    quantity_milli INTEGER NOT NULL DEFAULT 0,
    action TEXT NOT NULL,
    value INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_cancel_effects_key" ON "wl_inventory_cancel_effects" (effect_key);
CREATE INDEX "wl_inventory_cancel_effects_order" ON "wl_inventory_cancel_effects" (order_id);
