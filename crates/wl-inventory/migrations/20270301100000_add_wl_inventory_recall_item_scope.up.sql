-- Recalls of items not tracked by lot (wl-inventory §4 *Recall*, issue #236):
-- a scope row can name an item, a supplier and a received date range instead
-- of lot numbers, with an optional trace end date. The first receipt in the
-- range (found when the recall opens) starts the trace.
ALTER TABLE "wl_inventory_recall_scope" ADD COLUMN supplier_party_id INTEGER;
ALTER TABLE "wl_inventory_recall_scope" ADD COLUMN received_from TEXT;
ALTER TABLE "wl_inventory_recall_scope" ADD COLUMN received_to TEXT;
ALTER TABLE "wl_inventory_recall_scope" ADD COLUMN trace_end TEXT;
ALTER TABLE "wl_inventory_recall_scope" ADD COLUMN first_receipt_on TEXT;

-- The item of a line that names no lot (a line of an item not tracked by lot).
ALTER TABLE "wl_inventory_recall_lines" ADD COLUMN item_id INTEGER REFERENCES "wl_inventory_items" (id);
