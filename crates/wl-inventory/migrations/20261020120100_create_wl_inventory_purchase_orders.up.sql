-- Purchase orders (wl-inventory §3, §4 *Purchase order*).
CREATE TABLE "wl_inventory_purchase_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    supplier_party_id INTEGER NOT NULL,
    -- The delivery location.
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    currency TEXT NOT NULL DEFAULT 'AED',
    revision INTEGER NOT NULL DEFAULT 1,
    expected_date TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_purchase_orders_status" ON "wl_inventory_purchase_orders" (status);
CREATE INDEX "wl_inventory_purchase_orders_supplier" ON "wl_inventory_purchase_orders" (supplier_party_id);

CREATE TABLE "wl_inventory_purchase_order_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    purchase_order_id INTEGER NOT NULL REFERENCES "wl_inventory_purchase_orders" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    -- Thousandths of the item's base unit.
    quantity_milli INTEGER NOT NULL,
    -- The price of one base unit, in fils, before VAT.
    unit_price INTEGER NOT NULL,
    -- What was accepted so far; rejected goods are not in it.
    accepted_qty_milli INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_purchase_order_lines_position"
    ON "wl_inventory_purchase_order_lines" (purchase_order_id, position);
