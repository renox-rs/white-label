-- Chasing an aggregator's rider who does not arrive (06 F&B §7.2
-- `FNB.RUL.16`; BL-FNB-06): when the order became ready, when the kitchen
-- manager was told to chase the aggregator, and the task to record what
-- happened.
ALTER TABLE "fnb_orders" ADD COLUMN ready_at TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN pickup_alert_at TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN pickup_task_at TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN pickup_task_id INTEGER;
