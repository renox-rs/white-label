DROP INDEX IF EXISTS "wl_payments_payments_gateway_txn";
DROP INDEX IF EXISTS "wl_payments_payments_link";
ALTER TABLE "wl_payments_payments" DROP COLUMN late_confirmed_at;
ALTER TABLE "wl_payments_payments" DROP COLUMN failed_at;
ALTER TABLE "wl_payments_payments" DROP COLUMN failure_reason;
ALTER TABLE "wl_payments_payments" DROP COLUMN failure_kind;
ALTER TABLE "wl_payments_payments" DROP COLUMN subscription_id;
ALTER TABLE "wl_payments_payments" DROP COLUMN payment_link_id;
DROP TABLE IF EXISTS "wl_payments_payment_links";
