-- Where a sales order is delivered and the day a sale line is promised for
-- (BL-MFG-11 points 6 and 7).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_address TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_emirate TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_country TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_destination TEXT;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN promised_on DATE;
