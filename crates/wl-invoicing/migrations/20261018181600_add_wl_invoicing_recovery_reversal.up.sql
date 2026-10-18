-- Reversing a recovery (#70, part 4). A recovery whose payment is reversed
-- is marked, never deleted: when, and the idempotency key of the
-- unallocation that reversed it (NULL while it stands).
ALTER TABLE "wl_invoicing_recoveries" ADD COLUMN reversed_at TEXT;
ALTER TABLE "wl_invoicing_recoveries" ADD COLUMN reversed_key TEXT;
-- What the reversal did to the VAT side: `none`, `relief_restored`,
-- `relief_reinstated`, `adjustment_reduced`, `adjustment_excluded` or
-- `relief_created`.
ALTER TABLE "wl_invoicing_recoveries" ADD COLUMN reversed_effect TEXT;

CREATE UNIQUE INDEX "wl_invoicing_recoveries_reversed_key"
    ON "wl_invoicing_recoveries" (reversed_key)
    WHERE reversed_key IS NOT NULL;

-- A bad-debt relief raised again because a recovery that was already
-- included in a VAT summary was reversed: the recovery it answers.
ALTER TABLE "wl_invoicing_vat_adjustments" ADD COLUMN reverses_recovery_id INTEGER;

-- One bad-debt relief per invoice, besides the ones that reverse a
-- recovery (one per recovery).
DROP INDEX IF EXISTS "wl_invoicing_vat_adjustments_one_relief";
CREATE UNIQUE INDEX "wl_invoicing_vat_adjustments_one_relief"
    ON "wl_invoicing_vat_adjustments" (kind, source_type, source_id)
    WHERE kind = 'bad_debt_relief' AND reverses_recovery_id IS NULL;
CREATE UNIQUE INDEX "wl_invoicing_vat_adjustments_reverses_recovery"
    ON "wl_invoicing_vat_adjustments" (reverses_recovery_id)
    WHERE reverses_recovery_id IS NOT NULL;
