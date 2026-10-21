-- Dispatching a Return to supplier (wl-inventory §4 *Return to supplier*
-- `dispatch`): where the stock lines leave from, who dispatched it and when,
-- the return note, the price variance of a line and the quantity a
-- replacement line gave back to its order.
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN location_id INTEGER REFERENCES "wl_inventory_locations" (id);
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN dispatched_by INTEGER;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN dispatched_at TEXT;
-- The return note as a PDF document (ASCII text).
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN return_note TEXT;

-- (source receipt unit cost - average cost) x quantity, in fils: the purchase
-- price variance of a line taken from stock.
ALTER TABLE "wl_inventory_supplier_return_lines" ADD COLUMN variance INTEGER NOT NULL DEFAULT 0;
-- The quantity this line gave back to its purchase order line, in
-- thousandths; a line restores once.
ALTER TABLE "wl_inventory_supplier_return_lines" ADD COLUMN restored_milli INTEGER NOT NULL DEFAULT 0;
