DROP INDEX IF EXISTS "wl_invoicing_payer_credit_movements_refund";
ALTER TABLE "wl_invoicing_payer_credit_movements" DROP COLUMN approved_by;
ALTER TABLE "wl_invoicing_payer_credit_movements" DROP COLUMN refund_id;
