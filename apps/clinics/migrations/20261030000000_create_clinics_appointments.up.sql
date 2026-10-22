-- 05 Clinics (BL-CLN-01 base): the clinic's side of a wl-scheduling booking,
-- made from `BookingBooked`; the policy an insured appointment was booked
-- with is attached later (`CLN.INS.02`).
CREATE TABLE "clinics_appointments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    patient_party_id INTEGER,
    booker_party_id INTEGER NOT NULL,
    service_code TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    visit_date TEXT NOT NULL,
    policy_id INTEGER,
    cancelled_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_appointments_booking_id" ON "clinics_appointments" (booking_id);
CREATE INDEX "clinics_appointments_patient_party_id" ON "clinics_appointments" (patient_party_id);
CREATE INDEX "clinics_appointments_visit_date" ON "clinics_appointments" (visit_date);
