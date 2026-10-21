DROP TABLE "wl_invoicing_delivery_reviews";
DROP INDEX "wl_invoicing_disputes_source";
ALTER TABLE "wl_invoicing_disputes" DROP COLUMN source_id;
ALTER TABLE "wl_invoicing_disputes" DROP COLUMN source_type;
