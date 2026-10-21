DROP TABLE "wl_inventory_reorder_suggestions";
DROP TABLE "wl_inventory_preferred_supplier_requests";
DROP TABLE "wl_inventory_item_suppliers";
ALTER TABLE "wl_inventory_items" DROP COLUMN had_preferred_supplier;
ALTER TABLE "wl_inventory_items" DROP COLUMN reorder_policy;
ALTER TABLE "wl_inventory_items" DROP COLUMN reorder_max_milli;
ALTER TABLE "wl_inventory_items" DROP COLUMN reorder_point_milli;
