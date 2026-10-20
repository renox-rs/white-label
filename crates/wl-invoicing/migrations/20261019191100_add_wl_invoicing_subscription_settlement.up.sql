-- BL-INV-21 (#84): the pro rata settlement of a cancel is recorded with the
-- cancel itself, so a credit note or final invoice that failed is made on
-- the next try (a later cancel call or the daily check), never lost.
ALTER TABLE "wl_invoicing_subscriptions" ADD COLUMN settlement_due BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "wl_invoicing_subscriptions" ADD COLUMN settlement_paused_from TEXT;
ALTER TABLE "wl_invoicing_subscriptions" ADD COLUMN settlement_by INTEGER;
