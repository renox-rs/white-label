DROP INDEX IF EXISTS "wl_ticketing_charge_lines_ticket";
DROP TABLE IF EXISTS "wl_ticketing_charge_lines";
ALTER TABLE "wl_ticketing_tickets" DROP COLUMN invoice_id;
ALTER TABLE "wl_ticketing_categories" DROP COLUMN invoice_on;
