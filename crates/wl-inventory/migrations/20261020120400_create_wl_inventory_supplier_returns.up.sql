-- Returns to supplier (wl-inventory §3, §4 *Return to supplier*), as far as
-- a goods receipt makes them: the rejected goods of one receipt, refused at
-- the door or kept for collection.
CREATE TABLE "wl_inventory_supplier_returns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    supplier_party_id INTEGER NOT NULL,
    -- The goods receipt the goods were rejected at.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- `refused_at_door` or `kept_for_collection`.
    handling TEXT NOT NULL,
    -- The collection proof: the goods receipt for goods refused at the door.
    collection_proof TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_supplier_returns_source" ON "wl_inventory_supplier_returns" (source_type, source_id);
CREATE INDEX "wl_inventory_supplier_returns_status" ON "wl_inventory_supplier_returns" (status);

CREATE TABLE "wl_inventory_supplier_return_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    supplier_return_id INTEGER NOT NULL REFERENCES "wl_inventory_supplier_returns" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    quantity_milli INTEGER NOT NULL,
    -- The order's price of one base unit, in fils.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL,
    wanted_outcome TEXT NOT NULL DEFAULT 'replace',
    po_line_id INTEGER REFERENCES "wl_inventory_purchase_order_lines" (id),
    -- `rejected_at_receipt` or `from_stock`.
    origin TEXT NOT NULL DEFAULT 'rejected_at_receipt',
    receipt_line_id INTEGER REFERENCES "wl_inventory_goods_receipt_lines" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_supplier_return_lines_position"
    ON "wl_inventory_supplier_return_lines" (supplier_return_id, position);
