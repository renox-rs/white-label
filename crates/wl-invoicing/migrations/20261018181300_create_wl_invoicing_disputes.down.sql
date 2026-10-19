ALTER TABLE "wl_invoicing_invoices" DROP COLUMN dispute_grace_until;
ALTER TABLE "wl_invoicing_invoices" DROP COLUMN dispute_id;
DROP TABLE IF EXISTS "wl_invoicing_disputes";
