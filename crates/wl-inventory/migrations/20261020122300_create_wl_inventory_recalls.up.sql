-- Recalls (wl-inventory §3 *Recall*, §4 *Recall* `open`, issue #233): the
-- withdrawal of lots that may be unsafe. A recall names its scope (items and
-- the lot numbers or supplier lot numbers recalled), the lots found in that
-- scope, and the one stock hold (reason `recall`) that blocks them.
CREATE TABLE "wl_inventory_recalls" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    -- `supplier`, `authority`, `internal`, `complaint`.
    source TEXT NOT NULL,
    reason TEXT NOT NULL,
    -- The supplier behind a supplier recall, when known.
    supplier_party_id INTEGER,
    -- `open`, `traced`, `closed`.
    status TEXT NOT NULL DEFAULT 'open',
    stock_hold_id INTEGER REFERENCES "wl_inventory_stock_holds" (id),
    opened_by INTEGER,
    opened_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_recalls_status" ON "wl_inventory_recalls" (status);

-- What was recalled: one row per item and lot number or supplier lot number.
CREATE TABLE "wl_inventory_recall_scope" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL REFERENCES "wl_inventory_recalls" (id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_number TEXT,
    supplier_lot TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_recall_scope_recall_id" ON "wl_inventory_recall_scope" (recall_id);

-- The lots found in the scope, with or without stock left.
CREATE TABLE "wl_inventory_recall_lots" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL REFERENCES "wl_inventory_recalls" (id) ON DELETE CASCADE,
    lot_id INTEGER NOT NULL REFERENCES "wl_inventory_lots" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_recall_lots_lot"
    ON "wl_inventory_recall_lots" (recall_id, lot_id);
