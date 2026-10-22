-- Cancelling an order before and after acceptance (06 F&B §7.1 `cancel`,
-- `SAL.ACL.01`, `SAL.ACL.05`; BL-FNB-04): the reason and the responsible
-- party (business, aggregator, customer), who asked and whether the customer
-- did, the approval request that waits, and the flag that tells the payout
-- reconciliation to expect the aggregator's compensation.
ALTER TABLE "fnb_orders" ADD COLUMN cancel_party TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN cancel_reason_code TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN cancel_requested_by BIGINT;
ALTER TABLE "fnb_orders" ADD COLUMN cancel_request_id BIGINT;
ALTER TABLE "fnb_orders" ADD COLUMN cancelled_by BIGINT;
ALTER TABLE "fnb_orders" ADD COLUMN cancelled_by_customer BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "fnb_orders" ADD COLUMN cancelled_at TIMESTAMPTZ;
ALTER TABLE "fnb_orders" ADD COLUMN compensation_expected BOOLEAN NOT NULL DEFAULT FALSE;
