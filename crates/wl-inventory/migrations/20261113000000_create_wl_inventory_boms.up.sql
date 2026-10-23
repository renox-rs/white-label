-- Bills of materials with versions (wl-inventory §4 *BOM*, STK.BOM.01): the
-- components that make `output_milli` of an item, in draft until released.
CREATE TABLE "wl_inventory_boms" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The finished item the version makes.
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    -- 1, 2, 3 ... per item.
    version INTEGER NOT NULL,
    -- `draft`, `released`, `obsolete`.
    status TEXT NOT NULL DEFAULT 'draft',
    -- The quantity the components make, in thousandths (1000 = one unit).
    output_milli INTEGER NOT NULL DEFAULT 1000,
    -- The yield in basis points (10000 = every unit made is good).
    yield_bps INTEGER NOT NULL DEFAULT 10000,
    -- The standard hours of the output quantity, in thousandths of an hour.
    standard_milli_hours INTEGER NOT NULL DEFAULT 0,
    -- Rolled up at release: the materials of one unit at the average cost, in fils.
    material_cost_per_unit INTEGER NOT NULL DEFAULT 0,
    released_by INTEGER,
    released_at TEXT,
    obsoleted_at TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_boms_item_version" ON "wl_inventory_boms" (item_id, version);
CREATE INDEX "wl_inventory_boms_status" ON "wl_inventory_boms" (status);

CREATE TABLE "wl_inventory_bom_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bom_id INTEGER NOT NULL REFERENCES "wl_inventory_boms" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    -- The component item.
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    -- For the output quantity of the version, in thousandths of the base unit.
    quantity_milli INTEGER NOT NULL,
    -- The scrap allowance in basis points (500 = 5%).
    scrap_bps INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_bom_lines_position" ON "wl_inventory_bom_lines" (bom_id, position);
