-- Deciding a request to change the preferred supplier of an item
-- (`STK.ACL.07`, wl-inventory §3): who decided, when, why a rejection or a
-- cancellation, and the note of a request returned because the new supplier
-- is on hold at approval time.
ALTER TABLE "wl_inventory_preferred_supplier_requests" ADD COLUMN decided_by INTEGER;
ALTER TABLE "wl_inventory_preferred_supplier_requests" ADD COLUMN decided_at TEXT;
ALTER TABLE "wl_inventory_preferred_supplier_requests" ADD COLUMN decision_reason TEXT;
ALTER TABLE "wl_inventory_preferred_supplier_requests" ADD COLUMN returned_note TEXT;
