ALTER TABLE "logistics_client_terms" DROP COLUMN pickup_area;
ALTER TABLE "logistics_client_terms" DROP COLUMN pickup_address;
ALTER TABLE "logistics_parcels" DROP COLUMN refusal_reason;
DROP INDEX "logistics_shipments_return_of";
ALTER TABLE "logistics_shipments" DROP COLUMN return_of_id;
