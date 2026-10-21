DROP TABLE "wl_inventory_supplier_score_notes";
DROP TABLE "wl_inventory_supplier_scores";
DROP TABLE "wl_inventory_supplier_debit_notes";
ALTER TABLE "wl_inventory_purchase_orders" DROP COLUMN revision_issued_at;
ALTER TABLE "wl_inventory_purchase_orders" DROP COLUMN issued_at;
ALTER TABLE "wl_inventory_goods_receipts" DROP COLUMN late_entry;
ALTER TABLE "wl_inventory_goods_receipts" DROP COLUMN arrived_at;
