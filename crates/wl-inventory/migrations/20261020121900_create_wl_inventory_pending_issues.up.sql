-- Pending issues (wl-inventory §3 *Pending issue*, §3 *Negative stock*):
-- consumption another module posted automatically that stock could not
-- cover. It posts by itself when stock arrives, or the storekeeper clears it.
CREATE TABLE "wl_inventory_pending_issues" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The scope of the location: who clears it.
    scope_type TEXT,
    scope_id INTEGER,
    -- The short quantity, in thousandths of the base unit.
    quantity_milli INTEGER NOT NULL,
    -- The record that consumed (a visit, a sale, a work order).
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The record the cost is charged to.
    cost_object_type TEXT,
    cost_object_id INTEGER,
    recorded_at TEXT NOT NULL,
    -- `open`, `posted`, `cleared`.
    status TEXT NOT NULL DEFAULT 'open',
    -- The lot that covered it when it posted or was cleared.
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- The lot was chosen when stock arrived, not when the consumption happened.
    assigned_on_arrival INTEGER NOT NULL DEFAULT 0,
    -- The pending issue a partly covered one was split from.
    split_from_id INTEGER REFERENCES "wl_inventory_pending_issues" (id),
    -- `receipt`, `transfer`, `count`, `storekeeper` or `source_cancelled`.
    resolved_by TEXT,
    clear_reason TEXT,
    cleared_by INTEGER,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_pending_issues_status" ON "wl_inventory_pending_issues" (status);
CREATE INDEX "wl_inventory_pending_issues_location_id" ON "wl_inventory_pending_issues" (location_id, item_id);
CREATE INDEX "wl_inventory_pending_issues_source" ON "wl_inventory_pending_issues" (source_type, source_id);
