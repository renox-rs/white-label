-- Receiving a free replacement against a dispatched Return to supplier
-- (wl-inventory §4 *Goods receipt* `post`): the receipt names the return
-- instead of a purchase order, and each of its lines the return line it
-- replaces.
ALTER TABLE "wl_inventory_goods_receipts" ADD COLUMN supplier_return_id INTEGER REFERENCES "wl_inventory_supplier_returns" (id);
ALTER TABLE "wl_inventory_goods_receipt_lines" ADD COLUMN return_line_id INTEGER REFERENCES "wl_inventory_supplier_return_lines" (id);
CREATE INDEX "wl_inventory_goods_receipts_return" ON "wl_inventory_goods_receipts" (supplier_return_id);
