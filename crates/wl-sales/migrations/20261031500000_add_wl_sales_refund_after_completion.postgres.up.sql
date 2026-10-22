-- Refund after completion with nothing returned (BL-SAL-14, SAL.ACL.06).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN refunded_amount BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sales_return_lines" ADD COLUMN refund_value BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sales_returns" ADD COLUMN ticket_id BIGINT;
