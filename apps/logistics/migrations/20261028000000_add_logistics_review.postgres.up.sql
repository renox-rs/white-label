-- A delivered shipment under review of its delivery (03 Logistics §7.1
-- `open_review`, `close_review`; BL-LOG-09): the ticket of the dispute.
ALTER TABLE "logistics_shipments" ADD COLUMN review_ticket_id BIGINT;
