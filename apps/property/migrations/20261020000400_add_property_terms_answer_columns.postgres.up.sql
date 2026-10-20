-- The submitter's answer to "changes terms against the tenant?" on a renewal
-- or amendment draft, with the reason for a no (PRP.ACL.17, BL-PRP-04).
-- Not answered counts as yes: only a no is stored.
ALTER TABLE "property_leases" ADD COLUMN terms_answered_no BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "property_leases" ADD COLUMN terms_answer_reason TEXT;
