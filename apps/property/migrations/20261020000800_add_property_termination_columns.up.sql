-- Termination and its money (01 Property §7.1 `terminate`, `PRP.RUL.13`,
-- BL-PRP-11): the day the lease became `terminated`, the property manager's
-- choice for the credit the termination leaves (`refund` or `keep`), the net
-- amount credited for periods after the move-out date, and the day the
-- property manager was chased for the Ejari cancellation.
ALTER TABLE "property_leases" ADD COLUMN terminated_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN credit_choice TEXT NOT NULL DEFAULT 'refund';
ALTER TABLE "property_leases" ADD COLUMN termination_credit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "property_leases" ADD COLUMN ejari_chased_on TEXT;
