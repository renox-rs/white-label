-- Chargebacks (wl-payments §3 chargeback fields, §4 `charge_back`, §5.2a;
-- issue #97): when the dispute was opened, the gateway's fee and the outgoing
-- payment that records it, whether the evidence was submitted and how the
-- dispute ended. The evidence task of the accountant knows its deadline and
-- when it was reminded (`FIN.RUL.20`).
ALTER TABLE "wl_payments_payments" ADD COLUMN dispute_opened_on TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN dispute_fee INTEGER;
ALTER TABLE "wl_payments_payments" ADD COLUMN dispute_fee_payment_id INTEGER;
ALTER TABLE "wl_payments_payments" ADD COLUMN dispute_evidence_submitted_on TEXT;
ALTER TABLE "wl_payments_payments" ADD COLUMN dispute_outcome TEXT;
-- One payment per dispute reference: a dispute is recorded once.
CREATE UNIQUE INDEX "wl_payments_payments_dispute_reference"
    ON "wl_payments_payments" (dispute_reference)
    WHERE dispute_reference IS NOT NULL;

ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN due_on TEXT;
ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN reminded_on TEXT;
