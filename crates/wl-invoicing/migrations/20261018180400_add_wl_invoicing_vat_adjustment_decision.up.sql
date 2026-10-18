-- When FIN.RUL.14 proposed the adjustment (NULL before).
ALTER TABLE "wl_invoicing_vat_adjustments" ADD COLUMN proposed_at TEXT;
-- Who included or excluded it, and when (NULL before the decision).
ALTER TABLE "wl_invoicing_vat_adjustments" ADD COLUMN decided_by INTEGER;
ALTER TABLE "wl_invoicing_vat_adjustments" ADD COLUMN decided_at TEXT;
-- The reason given with the decision.
ALTER TABLE "wl_invoicing_vat_adjustments" ADD COLUMN decision_reason TEXT;
