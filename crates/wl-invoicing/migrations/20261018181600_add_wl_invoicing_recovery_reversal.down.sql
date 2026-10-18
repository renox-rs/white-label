DROP INDEX IF EXISTS "wl_invoicing_vat_adjustments_reverses_recovery";
DROP INDEX IF EXISTS "wl_invoicing_vat_adjustments_one_relief";
DELETE FROM "wl_invoicing_vat_adjustments" WHERE reverses_recovery_id IS NOT NULL;
CREATE UNIQUE INDEX "wl_invoicing_vat_adjustments_one_relief"
    ON "wl_invoicing_vat_adjustments" (kind, source_type, source_id)
    WHERE kind = 'bad_debt_relief';
ALTER TABLE "wl_invoicing_vat_adjustments" DROP COLUMN reverses_recovery_id;
DROP INDEX IF EXISTS "wl_invoicing_recoveries_reversed_key";
ALTER TABLE "wl_invoicing_recoveries" DROP COLUMN reversed_effect;
ALTER TABLE "wl_invoicing_recoveries" DROP COLUMN reversed_key;
ALTER TABLE "wl_invoicing_recoveries" DROP COLUMN reversed_at;
