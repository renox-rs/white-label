-- BL-SCH-06 (SCH.RUL.06, SCH.RUL.01): one escalation per booking per conflict,
-- and the reminders handled for a booking at its start.
ALTER TABLE "wl_scheduling_conflicts" ADD COLUMN escalated_at TEXT;

CREATE TABLE "wl_scheduling_reminders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL REFERENCES "wl_scheduling_bookings" (id),
    -- The hours before the start this reminder is for (24, 2).
    hours_before INTEGER NOT NULL,
    -- The start it was for: a reschedule makes the reminders new.
    for_start TEXT NOT NULL,
    -- sent, suppressed (the booking was in conflict) or skipped (a later
    -- reminder was already due).
    status TEXT NOT NULL,
    handled_at TEXT NOT NULL
);

CREATE INDEX "wl_scheduling_reminders_booking" ON "wl_scheduling_reminders" (booking_id);
CREATE UNIQUE INDEX "wl_scheduling_reminders_once" ON "wl_scheduling_reminders" (booking_id, hours_before, for_start);
