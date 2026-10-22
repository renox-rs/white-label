DROP INDEX IF EXISTS "wl_scheduling_prepayment_refunds_payment";
DROP INDEX IF EXISTS "wl_scheduling_prepayment_refunds_booking";
DROP TABLE IF EXISTS "wl_scheduling_prepayment_refunds";
ALTER TABLE "wl_scheduling_bookings" DROP COLUMN prepayment_path;
