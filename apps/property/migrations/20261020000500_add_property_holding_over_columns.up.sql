-- Holding over after the end date (PRP.RUL.03, PRP.RUL.14, BL-PRP-06): the
-- day the lease became `expired`, the start of the statutory renewal period
-- it holds over for now, the period a holding-over task was last opened for,
-- and the charges the extension created.
ALTER TABLE "property_leases" ADD COLUMN expired_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN holding_period_start TEXT;
ALTER TABLE "property_leases" ADD COLUMN holding_tasked_for TEXT;
ALTER TABLE "property_lease_charges" ADD COLUMN holding_over INTEGER NOT NULL DEFAULT 0;
