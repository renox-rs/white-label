-- Returns of recalled goods (wl-sales §3 *Sales return*, §4.5 `receive`, issue
-- #134): the lot a customer presents on a line, and the recall (a
-- wl-inventory record, `inventory.recall:N`) the return joined when the goods
-- were received.
ALTER TABLE "wl_sales_sales_return_lines" ADD COLUMN lot_id INTEGER;
ALTER TABLE "wl_sales_sales_returns" ADD COLUMN recall_id INTEGER;
