DROP INDEX IF EXISTS "wl_scheduling_reminders_once";
DROP INDEX IF EXISTS "wl_scheduling_reminders_booking";
DROP TABLE IF EXISTS "wl_scheduling_reminders";
ALTER TABLE "wl_scheduling_conflicts" DROP COLUMN escalated_at;
