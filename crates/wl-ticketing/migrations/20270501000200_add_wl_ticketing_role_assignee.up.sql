-- A ticket owned by a role in a scope until a holder takes it (BL-TKT-12,
-- wl-ticketing §8 `TicketRouter`): the role key, its name for people (with
-- the scope's), the permission that defines its holders when it is one, the
-- scope, and one key for the role queue.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_role TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_role_label TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_role_permission TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_scope_type TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_scope_id TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_role_key TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN assignee_role_display TEXT;
CREATE INDEX "wl_ticketing_tickets_assignee_role_key"
    ON "wl_ticketing_tickets" (assignee_role_key);
