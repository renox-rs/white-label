-- Recovering a failed refund (wl-payments §4 `retry`, `change_method`,
-- `cancel`, §5.5a, issue #100).

-- The details a changed method pays to: the IBAN of a transfer, or the name
-- on a cheque that is not the party's.
ALTER TABLE "wl_payments_refunds" ADD COLUMN payee_details TEXT;
-- The user who set them: new bank details need an approval by someone else
-- (`FIN.ACL.04`), so this user cannot approve the refund either.
ALTER TABLE "wl_payments_refunds" ADD COLUMN details_by INTEGER;
-- Why a failed refund was cancelled (the party is unreachable, or asks to
-- keep the credit).
ALTER TABLE "wl_payments_refunds" ADD COLUMN cancel_reason TEXT;

-- The refund a follow-up task is about (`change_method`: the accountant pays
-- a failed refund another way).
ALTER TABLE "wl_payments_followup_tasks" ADD COLUMN refund_id INTEGER;
