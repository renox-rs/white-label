-- A forced termination (01 Property §7.1 `force_terminate`, `PRP.ACL.19`,
-- BL-PRP-12): why (`abandoned` or `eviction_executed`), the legal basis the
-- property manager stated for an abandoned unit, and what the requester chose
-- for each later cheque (JSON: the ids to keep as evidence and to return).
ALTER TABLE "property_leases" ADD COLUMN force_reason TEXT;
ALTER TABLE "property_leases" ADD COLUMN force_basis TEXT;
ALTER TABLE "property_leases" ADD COLUMN force_cheques TEXT;
