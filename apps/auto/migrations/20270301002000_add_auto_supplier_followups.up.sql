-- 08 Auto: raising and recovering a supplier warranty claim (BL-AUT-11): when
-- the claim document was queued to the supplier, and when the purchaser and
-- the branch manager were told at 60 days (`AUT.RUL.08`, once).
ALTER TABLE "auto_supplier_claims" ADD COLUMN claim_sent_at TEXT;
ALTER TABLE "auto_supplier_claims" ADD COLUMN overdue_notified_at TEXT;
