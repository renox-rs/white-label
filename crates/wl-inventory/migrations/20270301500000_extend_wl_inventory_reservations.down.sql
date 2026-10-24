DROP INDEX IF EXISTS "wl_inventory_reservations_lot";
ALTER TABLE "wl_inventory_reservations" DROP COLUMN released_reason;
ALTER TABLE "wl_inventory_reservations" DROP COLUMN source_ref;
ALTER TABLE "wl_inventory_reservations" DROP COLUMN source;
ALTER TABLE "wl_inventory_reservations" DROP COLUMN lot_id;
