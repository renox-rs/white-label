DROP INDEX IF EXISTS "auto_settings_key";
DROP TABLE IF EXISTS "auto_settings";
ALTER TABLE "auto_job_cards" DROP COLUMN storage_invoiced_at;
ALTER TABLE "auto_job_cards" DROP COLUMN storage_invoice_id;
ALTER TABLE "auto_job_cards" DROP COLUMN storage_days;
ALTER TABLE "auto_job_cards" DROP COLUMN collect_manager_on;
ALTER TABLE "auto_job_cards" DROP COLUMN collect_second_on;
ALTER TABLE "auto_job_cards" DROP COLUMN collect_first_on;
