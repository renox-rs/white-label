DROP INDEX "wl_inventory_goods_receipts_return";
ALTER TABLE "wl_inventory_goods_receipt_lines" DROP COLUMN return_line_id;
ALTER TABLE "wl_inventory_goods_receipts" DROP COLUMN supplier_return_id;
