DROP INDEX IF EXISTS "wl_ticketing_compliance_flags_one_open";
DROP INDEX IF EXISTS "wl_ticketing_compliance_flags_subject";
DROP TABLE IF EXISTS "wl_ticketing_compliance_flags";
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN missed_notified_at;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN planned_due_date;
ALTER TABLE "wl_ticketing_categories" DROP COLUMN compliance_critical;
