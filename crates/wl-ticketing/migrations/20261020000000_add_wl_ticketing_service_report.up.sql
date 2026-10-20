-- The vendor's service report attached to a planned visit (01 Property
-- `PRP.ACL.15`, BL-PRP-24): a compliance-critical planned visit is resolved
-- only with its reference recorded.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN service_report TEXT;
