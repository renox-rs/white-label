-- Bookings and the slots they occupy (wl-scheduling §3, §4.1, SCH.BKG.01).
CREATE TABLE "wl_scheduling_bookings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- BKG-2026-0001.
    number TEXT NOT NULL,
    -- Who booked.
    party_id INTEGER NOT NULL,
    -- The patient, vehicle or student (a Subject), if any.
    subject_type TEXT,
    subject_id INTEGER,
    service_type_id INTEGER NOT NULL REFERENCES "wl_scheduling_service_types" (id),
    location_type TEXT,
    location_id INTEGER,
    meeting_link TEXT,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    -- staff, portal, website or phone.
    source TEXT NOT NULL DEFAULT 'staff',
    notes TEXT,
    status TEXT NOT NULL DEFAULT 'booked',
    -- A `requested` booking holds its slot until then; so does an unpaid one.
    hold_until TEXT,
    payment_pending BOOLEAN NOT NULL DEFAULT FALSE,
    -- The payment link of the prepayment (a Subject), if any.
    prepayment_type TEXT,
    prepayment_id INTEGER,
    created_by INTEGER,
    -- The conflict flag, one column per cause (§3).
    conflict_block BOOLEAN NOT NULL DEFAULT FALSE,
    conflict_eligibility BOOLEAN NOT NULL DEFAULT FALSE,
    -- The booking deposit (SCH.ACL.05): taken from the service type at booking.
    deposit_amount INTEGER NOT NULL DEFAULT 0,
    deposit_outcome TEXT,
    deposit_refund_id INTEGER,
    -- The cancellation (§4.1).
    cancel_code TEXT,
    cancel_initiated_by TEXT,
    cancel_by TEXT,
    cancel_note TEXT,
    cancel_stage TEXT,
    cancel_late BOOLEAN NOT NULL DEFAULT FALSE,
    cancelled_at TEXT,
    -- The vertical record that holds the work done (a Subject), if any.
    work_done_type TEXT,
    work_done_id INTEGER,
    confirmed_at TEXT,
    checked_in_at TEXT,
    started_at TEXT,
    completed_at TEXT,
    no_show_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_scheduling_bookings_number" ON "wl_scheduling_bookings" (number);
CREATE INDEX "wl_scheduling_bookings_status" ON "wl_scheduling_bookings" (status, starts_at);
CREATE INDEX "wl_scheduling_bookings_party" ON "wl_scheduling_bookings" (party_id);

-- The time each resource of a booking is occupied, buffers included. A
-- released row no longer counts; the booking's status decides the rest.
CREATE TABLE "wl_scheduling_booking_resources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL REFERENCES "wl_scheduling_bookings" (id),
    resource_id INTEGER NOT NULL REFERENCES "wl_scheduling_resources" (id),
    occupies_from TEXT NOT NULL,
    occupies_until TEXT NOT NULL,
    released BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX "wl_scheduling_booking_resources_booking" ON "wl_scheduling_booking_resources" (booking_id);
CREATE INDEX "wl_scheduling_booking_resources_resource" ON "wl_scheduling_booking_resources" (resource_id, occupies_from);
