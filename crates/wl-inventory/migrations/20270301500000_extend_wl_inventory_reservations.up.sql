-- The output lot of a work order serving a sale line is set aside in the same
-- table as the stock a confirmed order reserves (BL-MFG-02 point 5,
-- BL-MFG-03 beside BL-SAL-17): the lot, who asked and why a row was given
-- back. Rows that exist are the orders' (`order`).
ALTER TABLE "wl_inventory_reservations" ADD COLUMN lot_id INTEGER REFERENCES "wl_inventory_lots" (id);
ALTER TABLE "wl_inventory_reservations" ADD COLUMN source TEXT NOT NULL DEFAULT 'order';
ALTER TABLE "wl_inventory_reservations" ADD COLUMN source_ref TEXT;
ALTER TABLE "wl_inventory_reservations" ADD COLUMN released_reason TEXT;

CREATE INDEX "wl_inventory_reservations_lot" ON "wl_inventory_reservations" (lot_id);
