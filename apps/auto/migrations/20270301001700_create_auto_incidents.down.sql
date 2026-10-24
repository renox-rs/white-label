DROP INDEX IF EXISTS "auto_incidents_job_card_id";
DROP TABLE IF EXISTS "auto_incidents";
DROP INDEX IF EXISTS "auto_test_drives_job_card_id";
DROP TABLE IF EXISTS "auto_test_drives";
ALTER TABLE "auto_job_cards" DROP COLUMN incident_id;
ALTER TABLE "auto_job_cards" DROP COLUMN internal;
