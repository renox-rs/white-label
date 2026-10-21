DROP INDEX IF EXISTS "wl_sales_package_refusals_package";
DROP TABLE IF EXISTS "wl_sales_package_refusals";
DROP INDEX IF EXISTS "wl_sales_package_reopenings_source";
DROP TABLE IF EXISTS "wl_sales_package_reopenings";
ALTER TABLE "wl_sales_packages" DROP COLUMN collect_ticket_id;
ALTER TABLE "wl_sales_packages" DROP COLUMN reopened_at;
ALTER TABLE "wl_sales_packages" DROP COLUMN reopened_balance;
