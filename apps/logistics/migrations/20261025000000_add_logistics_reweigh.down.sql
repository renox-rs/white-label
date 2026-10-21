DROP TABLE "logistics_measurements";
DROP TABLE "logistics_client_terms";
ALTER TABLE "logistics_charges" DROP COLUMN billed_on;
ALTER TABLE "logistics_charges" DROP COLUMN ticket_id;
ALTER TABLE "logistics_charges" DROP COLUMN held;
ALTER TABLE "logistics_shipments" DROP COLUMN invoiced_on;
ALTER TABLE "logistics_shipments" DROP COLUMN reweigh_price;
ALTER TABLE "logistics_shipments" DROP COLUMN measured_weight_grams;
