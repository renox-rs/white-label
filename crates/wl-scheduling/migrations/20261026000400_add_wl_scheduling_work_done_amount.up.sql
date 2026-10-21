-- The work done before a cancellation after check-in, in fils, as the
-- vertical's record captured it (wl-scheduling §4.1, SCH.ACL.06).
ALTER TABLE "wl_scheduling_bookings" ADD COLUMN work_done_amount INTEGER NOT NULL DEFAULT 0;
