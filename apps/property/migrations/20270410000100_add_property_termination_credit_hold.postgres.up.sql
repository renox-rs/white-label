-- The termination credit held for the deposit settlement (01 Property §4.3
-- row "Early termination by the tenant", §7.1 `terminate`; BL-PRP-32): what
-- the credit notes of `terminate` left when a deposit was held, the credit
-- notes it sits on, and how it was used when the deposit was settled (on the
-- deductions, refunded, kept). The settlement statement of a forfeiture also
-- shows the credit applied before the deposit.
ALTER TABLE "property_leases" ADD COLUMN credit_held BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "property_leases" ADD COLUMN credit_notes_held TEXT;
ALTER TABLE "property_leases" ADD COLUMN credit_to_deductions BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "property_leases" ADD COLUMN credit_refunded BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "property_leases" ADD COLUMN credit_kept BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "property_leases" ADD COLUMN credit_settled_on DATE;
ALTER TABLE "property_deposit_settlements" ADD COLUMN credit_applied BIGINT NOT NULL DEFAULT 0;
