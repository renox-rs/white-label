ALTER TABLE "wl_payments_refunds" DROP COLUMN failure_reason;
ALTER TABLE "wl_payments_refunds" DROP COLUMN failure_kind;
ALTER TABLE "wl_payments_refunds" DROP COLUMN gateway_txn;
ALTER TABLE "wl_payments_refunds" DROP COLUMN source_kind;
DROP TABLE IF EXISTS "wl_payments_refund_attempts";
