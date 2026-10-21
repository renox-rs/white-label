DROP TABLE "logistics_statements";
DROP TABLE "logistics_remittance_offsets";
ALTER TABLE "logistics_remittances" DROP COLUMN message;
ALTER TABLE "logistics_remittances" DROP COLUMN offset_total;
ALTER TABLE "logistics_client_terms" DROP COLUMN netting_allowed;
