-- A refund of the payer credit balance (wl-payments #101, wl-invoicing §3
-- *Payer credit balance*, `FIN.ACL.16`): the movement names the refund it
-- belongs to and the user whose approval of that refund is the movement's.
ALTER TABLE "wl_invoicing_payer_credit_movements" ADD COLUMN refund_id INTEGER;
ALTER TABLE "wl_invoicing_payer_credit_movements" ADD COLUMN approved_by INTEGER;
CREATE INDEX "wl_invoicing_payer_credit_movements_refund"
    ON "wl_invoicing_payer_credit_movements" (refund_id);
