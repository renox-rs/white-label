-- A delivery whose source is held (wl-delivery §4.1, 03 Logistics §7.1 `hold`,
-- #303): the flag keeps it off a run until the source releases it.
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN held INTEGER NOT NULL DEFAULT 0;
