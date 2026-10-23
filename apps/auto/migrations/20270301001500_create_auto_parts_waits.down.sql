DROP INDEX IF EXISTS "auto_substitutes_job_card_id";
DROP TABLE IF EXISTS "auto_substitutes";
DROP INDEX IF EXISTS "auto_parts_waits_job_card_id";
DROP TABLE IF EXISTS "auto_parts_waits";
DROP INDEX IF EXISTS "auto_supply_periods_source";
DROP TABLE IF EXISTS "auto_supply_periods";
ALTER TABLE "auto_job_part_lines" DROP COLUMN expected_date;
ALTER TABLE "auto_job_part_lines" DROP COLUMN source;
