-- Whether SalesOrderConfirmed was emitted for the order: a release from a
-- hold at submit emits it, a release from a hold at dispatch does not
-- (BL-SAL-11).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN confirmed_announced INTEGER NOT NULL DEFAULT 0;
