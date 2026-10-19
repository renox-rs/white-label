DROP TABLE IF EXISTS "wl_invoicing_unallocations";
DROP INDEX IF EXISTS "wl_invoicing_allocations_active";
ALTER TABLE "wl_invoicing_allocations" DROP COLUMN removed_key;
ALTER TABLE "wl_invoicing_allocations" DROP COLUMN removed_reason;
ALTER TABLE "wl_invoicing_allocations" DROP COLUMN removed_at;
