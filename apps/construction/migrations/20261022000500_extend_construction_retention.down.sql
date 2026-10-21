DROP TABLE "construction_retention_notices";
ALTER TABLE "construction_retentions" DROP COLUMN announced_at;
ALTER TABLE "construction_retentions" DROP COLUMN bond_id;
ALTER TABLE "construction_retentions" DROP COLUMN invoice_id;
ALTER TABLE "construction_projects" DROP COLUMN retention_cap_notified_at;
ALTER TABLE "construction_projects" DROP COLUMN defects_liability_ends_on;
ALTER TABLE "construction_projects" DROP COLUMN completed_on;
ALTER TABLE "construction_projects" DROP COLUMN handover_certificate;
