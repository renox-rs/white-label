-- Preferred and alternative suppliers per item, and the reorder suggestions
-- that use them (wl-inventory §3 *Item supplier*, *Reorder suggestion*,
-- `STK.RUL.04`).
ALTER TABLE "wl_inventory_items" ADD COLUMN reorder_point_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_items" ADD COLUMN reorder_max_milli INTEGER NOT NULL DEFAULT 0;
-- `min_max`, `par` or `reorder_point_lead_time`.
ALTER TABLE "wl_inventory_items" ADD COLUMN reorder_policy TEXT NOT NULL DEFAULT 'min_max';
-- Set when the first preferred supplier is listed; any later change of the
-- preferred supplier is a request.
ALTER TABLE "wl_inventory_items" ADD COLUMN had_preferred_supplier INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "wl_inventory_item_suppliers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id) ON DELETE CASCADE,
    supplier_party_id INTEGER NOT NULL,
    -- `preferred` or `alternative`; at most one preferred per item.
    role TEXT NOT NULL,
    -- Among the alternatives, from 1; 0 for the preferred supplier.
    rank INTEGER NOT NULL DEFAULT 0,
    supplier_item_code TEXT,
    purchase_unit TEXT,
    lead_time_days INTEGER NOT NULL DEFAULT 0,
    -- In thousandths of the base unit; 0 for none.
    min_order_qty_milli INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    -- The document kinds kept on the supplier for the item, comma separated.
    certificate_kinds TEXT NOT NULL DEFAULT '',
    notes TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_item_suppliers_supplier"
    ON "wl_inventory_item_suppliers" (item_id, supplier_party_id);

-- A request to change who is the preferred supplier of an item (`STK.ACL.07`),
-- decided by a second person.
CREATE TABLE "wl_inventory_preferred_supplier_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id) ON DELETE CASCADE,
    -- The current preferred supplier.
    from_supplier_party_id INTEGER NOT NULL,
    -- The supplier to make preferred; none when the row is only deactivated.
    to_supplier_party_id INTEGER,
    -- `deactivate` or `replace`.
    kind TEXT NOT NULL,
    -- `pending`.
    status TEXT NOT NULL DEFAULT 'pending',
    requested_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_preferred_supplier_requests_item"
    ON "wl_inventory_preferred_supplier_requests" (item_id);

CREATE TABLE "wl_inventory_reorder_suggestions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The day of the run (Asia/Dubai).
    run_on TEXT NOT NULL,
    -- None when no supplier is usable and the purchaser picks one.
    supplier_party_id INTEGER,
    -- `preferred` or `alternative`.
    supplier_role TEXT,
    -- Why the preferred supplier was skipped, or why a purchaser switched.
    reason TEXT,
    -- The lead time of the supplier, for the policy that uses one.
    lead_time_days INTEGER,
    suggested_qty_milli INTEGER NOT NULL,
    -- The quantity before it was rounded up to the minimum order quantity.
    rounded_from_milli INTEGER,
    -- What the item had when the run looked.
    on_hand_milli INTEGER NOT NULL DEFAULT 0,
    -- The other suppliers shown, as JSON.
    alternatives TEXT NOT NULL DEFAULT '[]',
    -- The certificate kinds that are not valid for the chosen supplier, as JSON.
    warnings TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'open',
    switched_by INTEGER,
    switched_at TEXT,
    switch_reason TEXT,
    -- The supplier the suggestion was switched from.
    switched_from_party_id INTEGER,
    -- Whether that supplier was the item's preferred one.
    switched_from_preferred INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_reorder_suggestions_run"
    ON "wl_inventory_reorder_suggestions" (item_id, location_id, run_on);
CREATE INDEX "wl_inventory_reorder_suggestions_status"
    ON "wl_inventory_reorder_suggestions" (status);
