DROP TABLE IF EXISTS "wl_sales_back_order_serves";
ALTER TABLE "wl_sales_sale_lines" DROP COLUMN backordered_milli;
ALTER TABLE "wl_sales_sale_lines" DROP COLUMN reserved_milli;
ALTER TABLE "wl_sales_sales_orders" DROP COLUMN location_id;
