ALTER TABLE "fnb_expected_refunds" DROP COLUMN statement_id;
ALTER TABLE "fnb_orders" DROP COLUMN compensation_matched_at;
ALTER TABLE "fnb_orders" DROP COLUMN compensation_statement_id;
DROP TABLE "fnb_late_orders";
DROP TABLE "fnb_import_presets";
DROP TABLE "fnb_payout_reasons";
DROP TABLE "fnb_payout_lines";
DROP TABLE "fnb_payout_statements";
