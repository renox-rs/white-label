-- 05 Clinics appointment flags (`CLN.RUL.07`, base for BL-CLN-03; the same
-- flag carries `CLN.RUL.10`, BL-CLN-04): a mark on an appointment the front
-- desk sees on the day list and at check-in, with its cause, what raised it
-- and, once it no longer holds, the day and the reason it was cleared.
CREATE TABLE "clinics_appointment_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    appointment_id INTEGER NOT NULL REFERENCES "clinics_appointments" (id),
    cause TEXT NOT NULL,
    origin TEXT NOT NULL,
    authorisation_id INTEGER,
    detail TEXT,
    raised_on TEXT NOT NULL,
    cleared_on TEXT,
    clear_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_appointment_flags_booking_id" ON "clinics_appointment_flags" (booking_id);
CREATE INDEX "clinics_appointment_flags_cause" ON "clinics_appointment_flags" (cause);
