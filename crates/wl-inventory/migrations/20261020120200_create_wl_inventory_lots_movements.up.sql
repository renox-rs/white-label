-- Lots and stock movements (wl-inventory §3). A movement is immutable: it is
-- corrected by a reversing movement. A balance is derived from them.
CREATE TABLE "wl_inventory_lots" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    number TEXT NOT NULL,
    supplier_lot TEXT,
    -- The receipt (or production) the lot came from.
    source_type TEXT,
    source_id INTEGER,
    expiry_date TEXT,
    -- `quarantine`, `released`, `on_hold`, `expired`, `rejected`, `depleted`.
    status TEXT NOT NULL DEFAULT 'released',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_lots_number" ON "wl_inventory_lots" (item_id, number);

CREATE TABLE "wl_inventory_stock_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    movement_type TEXT NOT NULL,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    from_location_id INTEGER REFERENCES "wl_inventory_locations" (id),
    to_location_id INTEGER REFERENCES "wl_inventory_locations" (id),
    -- Thousandths of the base unit; always above 0.
    quantity_milli INTEGER NOT NULL,
    -- The cost of one base unit and the value of the movement, in fils.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    value INTEGER NOT NULL DEFAULT 0,
    cost_object_type TEXT,
    cost_object_id INTEGER,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    reason TEXT,
    user_id INTEGER,
    moved_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_stock_movements_item" ON "wl_inventory_stock_movements" (item_id);
CREATE INDEX "wl_inventory_stock_movements_to" ON "wl_inventory_stock_movements" (to_location_id);
CREATE INDEX "wl_inventory_stock_movements_from" ON "wl_inventory_stock_movements" (from_location_id);
CREATE INDEX "wl_inventory_stock_movements_source" ON "wl_inventory_stock_movements" (source_type, source_id);
