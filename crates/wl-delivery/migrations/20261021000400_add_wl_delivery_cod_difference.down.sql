DROP TABLE IF EXISTS "wl_delivery_approval_choices";
DROP TABLE IF EXISTS "wl_delivery_cod_reasons";
ALTER TABLE "wl_delivery_proofs" DROP COLUMN approval_request_id;
ALTER TABLE "wl_delivery_proofs" DROP COLUMN absorbed_by;
ALTER TABLE "wl_delivery_proofs" DROP COLUMN cod_difference_reason;
