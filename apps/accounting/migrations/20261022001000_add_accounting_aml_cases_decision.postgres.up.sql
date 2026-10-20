-- 04 Accounting §7.1 *AML case* `report` (BL-ACC-11): the MLRO's recorded
-- decision on the engagement after a report (continue, suspend, exit).
ALTER TABLE "accounting_aml_cases" ADD COLUMN engagement_decision TEXT;
ALTER TABLE "accounting_aml_cases" ADD COLUMN decision_note TEXT;
ALTER TABLE "accounting_aml_cases" ADD COLUMN decided_by BIGINT;
ALTER TABLE "accounting_aml_cases" ADD COLUMN decided_on DATE;
