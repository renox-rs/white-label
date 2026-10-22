-- Reminding the deciders of an open stock hold and escalating it
-- (wl-inventory §6 `STK.RUL.07`, issue #232): how many reminders and
-- escalations went out and when the last of each did, so each is sent once
-- per period however often the hourly job runs.
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN reminders_sent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN last_reminded_at TEXT;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN escalations_sent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_stock_holds" ADD COLUMN last_escalated_at TEXT;
