-- The waiting list (wl-scheduling §3, §4.2, SCH.BKG.01): requests for an
-- earlier or unavailable slot. An offered entry holds its slot until the
-- offer lapses.
CREATE TABLE "wl_scheduling_waiting_entries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    subject_type TEXT,
    subject_id INTEGER,
    service_type_id INTEGER NOT NULL REFERENCES "wl_scheduling_service_types" (id),
    -- NULL = any resource that delivers the service.
    preferred_resource_id INTEGER,
    -- The window the party can come in; NULL = any time.
    window_from TEXT,
    window_until TEXT,
    -- The last day the entry is looked at.
    expires_on TEXT,
    status TEXT NOT NULL DEFAULT 'waiting',
    -- The slot on offer, and until when it is held for the party.
    offered_resource_id INTEGER,
    offered_from TEXT,
    offered_until TEXT,
    offer_hold_until TEXT,
    -- The booking the party took.
    booking_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_scheduling_waiting_entries_status" ON "wl_scheduling_waiting_entries" (status, service_type_id);
