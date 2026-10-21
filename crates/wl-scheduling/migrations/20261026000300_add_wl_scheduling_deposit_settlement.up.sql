-- How a booking deposit was settled and who waived a forfeiture
-- (wl-scheduling §4.1, SCH.ACL.05). The amounts are in fils and add up to the
-- deposit once it is settled.
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_applied INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_refunded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_forfeited INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_waived_by INTEGER;
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_waived_at TEXT;
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN deposit_waive_reason TEXT;
