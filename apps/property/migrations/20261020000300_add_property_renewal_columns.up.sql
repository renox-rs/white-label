-- Renewal window and notice deadline (PRP.RUL.01, PRP.RUL.09, BL-PRP-03): the
-- day the lease entered the window, and the day each deadline warning was
-- sent (one per lease per offset).
ALTER TABLE "property_leases" ADD COLUMN renewal_flagged_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN deadline_warned_first_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN deadline_warned_second_on TEXT;
