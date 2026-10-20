-- Goods receipts (wl-inventory §3, §4 *Goods receipt*): a delivery against a
-- purchase order with the accepted and the rejected quantity of every line.
CREATE TABLE "wl_inventory_goods_receipts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- From the `goods_receipt` series at `post`; NULL while a draft.
    number TEXT UNIQUE,
    purchase_order_id INTEGER REFERENCES "wl_inventory_purchase_orders" (id),
    supplier_party_id INTEGER NOT NULL,
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The scope of the location, copied: who may post and who approves.
    scope_type TEXT,
    scope_id INTEGER,
    delivery_note TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER,
    posted_by INTEGER,
    posted_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_goods_receipts_status" ON "wl_inventory_goods_receipts" (status);
CREATE INDEX "wl_inventory_goods_receipts_po" ON "wl_inventory_goods_receipts" (purchase_order_id);

CREATE TABLE "wl_inventory_goods_receipt_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    goods_receipt_id INTEGER NOT NULL REFERENCES "wl_inventory_goods_receipts" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    po_line_id INTEGER REFERENCES "wl_inventory_purchase_order_lines" (id),
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    -- Thousandths of the base unit: received = accepted + rejected.
    received_qty_milli INTEGER NOT NULL DEFAULT 0,
    accepted_qty_milli INTEGER NOT NULL DEFAULT 0,
    rejected_qty_milli INTEGER NOT NULL DEFAULT 0,
    -- `temperature`, `quality`, `damaged`... required for a rejected quantity.
    rejection_reason TEXT,
    -- `refused_at_door` or `kept_for_collection`.
    rejected_handling TEXT,
    -- `replace` or `credit`.
    wanted_outcome TEXT NOT NULL DEFAULT 'replace',
    lot_number TEXT,
    expiry_date TEXT,
    -- A reading or the reference of a photo, when the reason needs one.
    evidence TEXT,
    -- The order's price of one base unit, in fils.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    -- Set at `post` for an accepted lot-tracked line.
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_goods_receipt_lines_position"
    ON "wl_inventory_goods_receipt_lines" (goods_receipt_id, position);
