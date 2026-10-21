-- Deciding the lines of a stock hold, closing it and cancelling it
-- (wl-inventory §4 *Stock hold*, issue #231): each line has a status of its
-- own (a state machine, so a decision can wait for an approval), the scope of
-- its location (who may decide it) and, for `return_to_supplier`, the draft
-- return it was added to. The hold keeps the scope of the location it is on,
-- when it was placed, and why and by whom it was cancelled.
ALTER TABLE "wl_inventory_stock_hold_lines" ADD COLUMN status TEXT NOT NULL DEFAULT 'undecided';
ALTER TABLE "wl_inventory_stock_hold_lines" ADD COLUMN scope_type TEXT;
ALTER TABLE "wl_inventory_stock_hold_lines" ADD COLUMN scope_id INTEGER;
ALTER TABLE "wl_inventory_stock_hold_lines" ADD COLUMN supplier_return_id INTEGER;

UPDATE "wl_inventory_stock_hold_lines"
    SET status = CASE outcome
        WHEN 'release' THEN 'released'
        WHEN 'discard' THEN 'discarded'
        WHEN 'return_to_supplier' THEN 'returned_to_supplier'
        ELSE 'undecided' END
    WHERE outcome IS NOT NULL;
UPDATE "wl_inventory_stock_hold_lines"
    SET scope_type = (SELECT l.scope_type FROM "wl_inventory_locations" l WHERE l.id = location_id),
        scope_id = (SELECT l.scope_id FROM "wl_inventory_locations" l WHERE l.id = location_id);

CREATE INDEX "wl_inventory_stock_hold_lines_status" ON "wl_inventory_stock_hold_lines" (status);

ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN scope_type TEXT;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN scope_id INTEGER;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN placed_at TEXT;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN cancel_reason TEXT;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN cancelled_by INTEGER;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN closed_at TEXT;

UPDATE "wl_inventory_stock_holds" SET placed_at = created_at;
UPDATE "wl_inventory_stock_holds"
    SET scope_type = (SELECT l.scope_type FROM "wl_inventory_locations" l WHERE l.id = location_id),
        scope_id = (SELECT l.scope_id FROM "wl_inventory_locations" l WHERE l.id = location_id)
    WHERE location_id IS NOT NULL;
