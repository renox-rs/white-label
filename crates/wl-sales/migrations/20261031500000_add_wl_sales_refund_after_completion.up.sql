-- Refund after completion with nothing returned (BL-SAL-14, SAL.ACL.06): what
-- the returns of an order refunded so far (the order's amount refunded, fils),
-- the value of each return line (the cap per line is a value: a partial
-- refund of a line is allowed) and the complaint ticket a return was raised
-- from.
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN refunded_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sales_return_lines" ADD COLUMN refund_value INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sales_returns" ADD COLUMN ticket_id INTEGER;
