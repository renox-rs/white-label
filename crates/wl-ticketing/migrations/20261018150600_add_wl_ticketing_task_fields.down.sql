DROP TABLE IF EXISTS "wl_ticketing_comments";
DROP INDEX IF EXISTS "wl_ticketing_tickets_source_key";
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN source_key;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN due_at;
