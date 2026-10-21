-- Delivering part of a multi-parcel shipment and returning the refused
-- parcels on a linked return shipment (03 Logistics §4.1, §6, §7.1
-- `create_return`; BL-LOG-07).
-- The shipment a return shipment was made from.
ALTER TABLE "logistics_shipments" ADD COLUMN return_of_id BIGINT;
CREATE INDEX "logistics_shipments_return_of" ON "logistics_shipments" (return_of_id);
-- Why the recipient refused the parcel (a refusal reason code).
ALTER TABLE "logistics_parcels" ADD COLUMN refusal_reason TEXT;
-- Where a client's refused parcels go back to (a stand-in for the address
-- on the client's party record).
ALTER TABLE "logistics_client_terms" ADD COLUMN pickup_address TEXT;
ALTER TABLE "logistics_client_terms" ADD COLUMN pickup_area TEXT;
