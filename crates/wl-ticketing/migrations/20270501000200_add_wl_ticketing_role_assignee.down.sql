DROP INDEX IF EXISTS "wl_ticketing_tickets_assignee_role_key";
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_role_display;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_role_key;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_scope_id;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_scope_type;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_role_permission;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_role_label;
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN assignee_role;
