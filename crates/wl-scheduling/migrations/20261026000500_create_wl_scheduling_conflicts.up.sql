-- The causes of a booking's conflict (wl-scheduling §3 Booking: conflict,
-- §5.5; BL-SCH-05): one row per booking and cause, so that removing the block
-- clears only the conflicts that block caused.
CREATE TABLE "wl_scheduling_conflicts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL REFERENCES "wl_scheduling_bookings" (id),
    -- The resource that is not available.
    resource_id INTEGER NOT NULL REFERENCES "wl_scheduling_resources" (id),
    -- block or eligibility.
    cause TEXT NOT NULL,
    -- The availability block that caused it (cause block).
    block_id INTEGER,
    -- The expiring qualification or document (cause eligibility).
    item TEXT,
    raised_at TEXT NOT NULL,
    cleared_at TEXT,
    -- How it ended: block_removed, reassign, reschedule or cancel.
    cleared_by TEXT,
    -- When the party was last sent the offer of other times.
    offered_at TEXT
);

CREATE INDEX "wl_scheduling_conflicts_booking" ON "wl_scheduling_conflicts" (booking_id);
CREATE INDEX "wl_scheduling_conflicts_block" ON "wl_scheduling_conflicts" (block_id);
