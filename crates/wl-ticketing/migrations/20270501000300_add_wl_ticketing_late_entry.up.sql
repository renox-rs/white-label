-- A ticket entered late keeps the time the problem was reported (BL-TKT-13,
-- wl-kernel §5.13): its business time, the late-entry flag with the reason
-- and outage reference, and the kernel's late entry it was recorded under.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN reported_at TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN late_entry INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN late_reason TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN outage_reference TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN late_entry_id INTEGER;
