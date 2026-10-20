-- Items and stock locations (wl-inventory §3), as far as receiving needs.
CREATE TABLE "wl_inventory_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    -- `stocked`, `non_stocked`, `consumable`, `finished`, `component`, `service`.
    item_type TEXT NOT NULL DEFAULT 'stocked',
    -- The unit all quantities are kept in (kg, pcs, m).
    base_unit TEXT NOT NULL,
    lot_tracked INTEGER NOT NULL DEFAULT 0,
    -- New lots of the item start in `quarantine`.
    quarantine_new_lots INTEGER NOT NULL DEFAULT 0,
    shelf_life_days INTEGER,
    -- Moving weighted average cost of one base unit, in fils.
    average_cost INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_inventory_locations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    location_type TEXT NOT NULL DEFAULT 'store',
    parent_id INTEGER REFERENCES "wl_inventory_locations" (id),
    -- The building, project or branch the location is in.
    scope_type TEXT,
    scope_id INTEGER,
    -- `block` or `allow_consumption`.
    negative_stock_policy TEXT NOT NULL DEFAULT 'block',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_locations_scope" ON "wl_inventory_locations" (scope_type, scope_id);
