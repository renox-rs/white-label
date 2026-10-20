-- Chasing returns to supplier that stay `dispatched` (wl-inventory §6
-- `STK.RUL.08`): the day of the last notice and how many were sent.
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN last_chased_on TEXT;
ALTER TABLE "wl_inventory_supplier_returns" ADD COLUMN chase_count INTEGER NOT NULL DEFAULT 0;
