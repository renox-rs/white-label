-- Where a sales order is delivered (wl-sales §3 Sales order; BL-MFG-11 point 6,
-- 07 Manufacturing): the delivery address with its emirate and country, and
-- the answer domestic or export (the VAT category of the address), which the
-- vertical's line rules and the dispatch guard read. And the day a sale line
-- is promised for, which the look-ahead of an expiring document reads
-- (BL-MFG-11 point 7).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_address TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_emirate TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_country TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN delivery_destination TEXT;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN promised_on TEXT;
