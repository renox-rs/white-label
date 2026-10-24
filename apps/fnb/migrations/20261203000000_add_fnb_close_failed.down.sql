DROP TABLE "fnb_refund_requests";
ALTER TABLE "fnb_orders" DROP COLUMN close_reason;
ALTER TABLE "fnb_orders" DROP COLUMN closed_at;
ALTER TABLE "fnb_orders" DROP COLUMN wastage_posted;
ALTER TABLE "fnb_orders" DROP COLUMN wastage_ref;
ALTER TABLE "fnb_orders" DROP COLUMN refund_id;
ALTER TABLE "fnb_orders" DROP COLUMN refund_amount;
ALTER TABLE "fnb_orders" DROP COLUMN refund_decision;
