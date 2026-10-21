-- Purchase requests and stock counts, as far as a vertical that stops a
-- project needs them (wl-inventory §3 *Purchase request*, *Stock count*,
-- §4; 02 Construction §7.1 *Project* `terminate`).
CREATE TABLE "wl_inventory_purchase_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The location the goods are needed at.
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The cost object, as a subject (`construction.cost_code:3`).
    cost_object TEXT,
    needed_by TEXT,
    requester_id INTEGER,
    -- `draft`, `submitted`, `approved`, `ordered`, `rejected` or `cancelled`.
    status TEXT NOT NULL DEFAULT 'draft',
    cancel_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_purchase_requests_location"
    ON "wl_inventory_purchase_requests" (location_id);

CREATE TABLE "wl_inventory_stock_counts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- Why it was opened.
    reason TEXT,
    -- The record that asked for it (`construction.project:4`).
    source TEXT,
    blind INTEGER NOT NULL DEFAULT 0,
    opened_by INTEGER,
    opened_at TEXT,
    -- `open`, `submitted`, `recount`, `approved` or `cancelled`.
    status TEXT NOT NULL DEFAULT 'open',
    cancel_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_stock_counts_location"
    ON "wl_inventory_stock_counts" (location_id);
