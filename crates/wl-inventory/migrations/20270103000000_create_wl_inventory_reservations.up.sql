-- Stock set aside for a sales order line (wl-inventory §3 *Stock balance*:
-- reserved), BL-SAL-11 point 2 (a released order reserves its lines when
-- `SalesOrderConfirmed` comes), BL-MFG-02 point 5 and BL-MFG-03 (the output
-- lot of a work order serving a sale line). One row per order, line, source
-- and lot; a row given back keeps its history (`status`).
CREATE TABLE "wl_inventory_reservations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    -- The lot, or 0 when the reservation has none (makes the key unique).
    lot_key INTEGER NOT NULL DEFAULT 0,
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- Thousandths of the base unit.
    quantity_milli INTEGER NOT NULL,
    -- The sales order and its line (their ids in wl-sales).
    order_id INTEGER NOT NULL,
    sale_line_id INTEGER NOT NULL,
    -- `order` (the confirmed order) or `work_order` (the output of one).
    source TEXT NOT NULL,
    -- The record that asked, e.g. `manufacturing.work_order:5`.
    source_ref TEXT,
    -- `active`, `released` (given back) or `consumed` (a dispatch picked it).
    status TEXT NOT NULL DEFAULT 'active',
    released_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_reservations_key" ON "wl_inventory_reservations" (order_id, sale_line_id, source, lot_key);
CREATE INDEX "wl_inventory_reservations_stock" ON "wl_inventory_reservations" (item_id, location_id);
CREATE INDEX "wl_inventory_reservations_order" ON "wl_inventory_reservations" (order_id);
CREATE INDEX "wl_inventory_reservations_lot" ON "wl_inventory_reservations" (lot_id);
