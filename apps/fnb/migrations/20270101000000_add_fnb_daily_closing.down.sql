DROP TABLE "fnb_daily_closings";
DROP INDEX "fnb_orders_business_date";
ALTER TABLE "fnb_orders" DROP COLUMN business_date;
ALTER TABLE "fnb_outlets" DROP COLUMN pending_cutoff_from;
ALTER TABLE "fnb_outlets" DROP COLUMN pending_cutoff_minutes;
ALTER TABLE "fnb_outlets" DROP COLUMN cutoff_minutes;
