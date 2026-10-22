-- The status a lot had when a stock hold on whole lots put it `on_hold`
-- (wl-inventory §4 *Lot* `hold`), kept so `unhold` can give it back.
ALTER TABLE "wl_inventory_lots" ADD COLUMN status_before_hold TEXT;
