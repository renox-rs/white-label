-- Stock holds (wl-inventory §3 *Stock hold*, §4 *Stock hold* `place` and
-- `add_lines`): stock blocked without moving it, whole lots or the stock at
-- one location.
CREATE TABLE "wl_inventory_stock_holds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    -- `recall`, `quality`, `temperature`, `complaint`, `other`.
    reason TEXT NOT NULL,
    -- The record that caused it (a recall, a ticket, an NCR, a reading).
    reference_type TEXT,
    reference_id INTEGER,
    -- `lots` or `location`.
    scope_kind TEXT NOT NULL,
    location_id INTEGER REFERENCES "wl_inventory_locations" (id),
    -- `open`, `closed`, `cancelled`.
    status TEXT NOT NULL DEFAULT 'open',
    placed_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_stock_holds_status" ON "wl_inventory_stock_holds" (status);

CREATE TABLE "wl_inventory_stock_hold_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stock_hold_id INTEGER NOT NULL REFERENCES "wl_inventory_stock_holds" (id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- Thousandths of the base unit; always above 0.
    quantity_milli INTEGER NOT NULL,
    -- `release`, `discard` or `return_to_supplier` once decided.
    outcome TEXT,
    decided_by INTEGER,
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_stock_hold_lines_stock_hold_id" ON "wl_inventory_stock_hold_lines" (stock_hold_id);

-- The items a hold on a location is limited to; none means every item.
CREATE TABLE "wl_inventory_stock_hold_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stock_hold_id INTEGER NOT NULL REFERENCES "wl_inventory_stock_holds" (id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_stock_hold_items_item"
    ON "wl_inventory_stock_hold_items" (stock_hold_id, item_id);
