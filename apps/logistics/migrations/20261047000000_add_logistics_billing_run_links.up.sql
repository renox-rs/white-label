-- The monthly run that billed a shipment or a charge, so that a draft
-- invoice that is discarded releases what it billed (03 Logistics §4.1 step
-- 12; BL-LOG-20).
ALTER TABLE "logistics_shipments" ADD COLUMN billing_run_id INTEGER;
ALTER TABLE "logistics_charges" ADD COLUMN billing_run_id INTEGER;
