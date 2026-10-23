-- Stock set aside for a line of a sales order (wl-sales §4.2 *Back-orders*,
-- wl-inventory §3 *Stock balance* "reserved"; BL-SAL-17). Reserved stock is
-- not available to any other picking or consumption. One row per reservation
-- made; giving back lowers the quantity (a row at 0 is spent).
CREATE TABLE "wl_inventory_reservations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The record the stock is set aside for, e.g. sales.sales_order:5, and
    -- its line.
    owner_type TEXT NOT NULL,
    owner_id INTEGER NOT NULL,
    line_id INTEGER NOT NULL DEFAULT 0,
    quantity_milli INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_reservations_place"
    ON "wl_inventory_reservations" (item_id, location_id);
CREATE INDEX "wl_inventory_reservations_owner"
    ON "wl_inventory_reservations" (owner_type, owner_id);
