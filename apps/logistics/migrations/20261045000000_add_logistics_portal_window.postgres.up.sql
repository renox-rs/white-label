-- The client's portal window after its contract ends (03 Logistics §3.3
-- `POR.ACL.01`; BL-LOG-26): read-only for 90 days after this day, then closed.
ALTER TABLE "logistics_client_terms" ADD COLUMN contract_ended_on DATE;
