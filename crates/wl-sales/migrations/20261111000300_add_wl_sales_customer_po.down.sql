DROP INDEX IF EXISTS "wl_sales_order_po_keys_key";
DROP INDEX IF EXISTS "wl_sales_order_po_keys_order";
DROP TABLE IF EXISTS "wl_sales_order_po_keys";
ALTER TABLE "wl_sales_sales_orders" DROP COLUMN call_off;
ALTER TABLE "wl_sales_sales_orders" DROP COLUMN customer_po;
