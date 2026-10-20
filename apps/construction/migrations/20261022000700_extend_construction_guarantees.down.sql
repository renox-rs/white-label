DROP TABLE "construction_guarantee_documents";
ALTER TABLE "construction_projects" DROP COLUMN advance_recovered;
ALTER TABLE "construction_projects" DROP COLUMN advance_paid;
ALTER TABLE "construction_guarantees" DROP COLUMN reminded_for;
ALTER TABLE "construction_guarantees" DROP COLUMN beneficiary;
