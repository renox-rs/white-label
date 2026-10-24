-- What a production report needs to be voided (BL-MFG-04): the plant and the
-- line it was reported on (where the void is approved), the output lot and the
-- store the good quantity went to, the cost of a unit it was posted at, and
-- when the reversing stock movements were posted after a void.
ALTER TABLE "manufacturing_production_reports" ADD COLUMN plant_id BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "manufacturing_production_reports" ADD COLUMN line_id BIGINT;
ALTER TABLE "manufacturing_production_reports" ADD COLUMN lot_id BIGINT;
ALTER TABLE "manufacturing_production_reports" ADD COLUMN location_id BIGINT;
ALTER TABLE "manufacturing_production_reports" ADD COLUMN unit_cost BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "manufacturing_production_reports" ADD COLUMN stock_reversed_at TIMESTAMPTZ;
UPDATE "manufacturing_production_reports" SET
    plant_id = (SELECT w.plant_id FROM "manufacturing_work_orders" w WHERE w.id = work_order_id),
    line_id = (SELECT w.line_id FROM "manufacturing_work_orders" w WHERE w.id = work_order_id),
    lot_id = (SELECT w.output_lot_id FROM "manufacturing_work_orders" w WHERE w.id = work_order_id),
    unit_cost = (SELECT w.std_unit_cost FROM "manufacturing_work_orders" w WHERE w.id = work_order_id),
    location_id = (SELECT l.finished_location_id FROM "manufacturing_lines" l
                   WHERE l.id = (SELECT w.line_id FROM "manufacturing_work_orders" w WHERE w.id = work_order_id));
