-- Returns of recalled goods (issue #134): the recall (`inventory.recall:N`) a
-- return line is to join, written before the goods are received, and when the
-- line joined it. A line pending and not joined is retried by the scheduled
-- task, so goods are never left off the hold when taking them in failed.
ALTER TABLE "wl_sales_sales_return_lines" ADD COLUMN recall_pending_id INTEGER;
ALTER TABLE "wl_sales_sales_return_lines" ADD COLUMN recall_joined_at TEXT;
