-- The failed own-rider delivery of an order (06 F&B §7.1 `fail_delivery`;
-- BL-FNB-01): the reason mapped from wl-delivery's, the reason wl-delivery
-- kept, the side it is on, who is proposed as responsible, and when.
ALTER TABLE "fnb_orders" ADD COLUMN failure_reason TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN delivery_reason TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN failure_side TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN failure_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "fnb_orders" ADD COLUMN failed_at TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN responsible_party TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN responsible_note TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN responsible_changed_by INTEGER;
