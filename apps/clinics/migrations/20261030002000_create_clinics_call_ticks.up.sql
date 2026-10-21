-- 05 Clinics, the call list when the SMS or WhatsApp connection is down
-- (BL-CLN-15, #363, 05 section 8): the front desk ticks an appointment as
-- called. A tick belongs to one outage, so a new outage starts a new list.
CREATE TABLE "clinics_call_ticks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    outage_since TEXT NOT NULL,
    called_by INTEGER,
    called_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_call_ticks_booking_outage" ON "clinics_call_ticks" (booking_id, outage_since);
