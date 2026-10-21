-- Transfers between two locations in two steps (wl-inventory §4 *Transfer*):
-- `send` takes the stock to the in-transit location, `receive` puts it at
-- the destination.
CREATE TABLE "wl_inventory_transfers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    from_location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    to_location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The scope of each end: who may send and who may receive.
    from_scope_type TEXT,
    from_scope_id INTEGER,
    to_scope_type TEXT,
    to_scope_id INTEGER,
    transit_location_id INTEGER REFERENCES "wl_inventory_locations" (id),
    -- `draft`, `in_transit`, `received`, `cancelled`.
    status TEXT NOT NULL DEFAULT 'draft',
    sent_by INTEGER,
    sent_at TEXT,
    received_by INTEGER,
    received_at TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_transfers_status" ON "wl_inventory_transfers" (status);

CREATE TABLE "wl_inventory_transfer_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transfer_id INTEGER NOT NULL REFERENCES "wl_inventory_transfers" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- Thousandths of the base unit; always above 0.
    quantity_milli INTEGER NOT NULL,
    -- The average cost of one base unit when it was sent, in fils.
    unit_cost INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_transfer_lines_position"
    ON "wl_inventory_transfer_lines" (transfer_id, position);
