-- The till that holds the cash of a till session handover (BL-PAY-19, #112):
-- the holder of such a handover is the till, not the cashier.
ALTER TABLE "wl_payments_cash_handovers" ADD COLUMN holder_till TEXT;
CREATE UNIQUE INDEX "wl_payments_cash_handovers_till_session"
    ON "wl_payments_cash_handovers" (period_type, period_id)
    WHERE holder_till IS NOT NULL AND kind = 'period';
