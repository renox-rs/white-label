-- 05 Clinics (BL-CLN-01 base): the visit a check-in opens (`CLN.ENC.01`), one
-- per wl-scheduling booking, with the machine's status. The coverage columns
-- (eligibility result, outcome, consent) come with the coverage outcome.
CREATE TABLE "clinics_visits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    appointment_id INTEGER REFERENCES "clinics_appointments" (id),
    patient_party_id INTEGER,
    policy_id INTEGER,
    service_code TEXT NOT NULL,
    visit_date TEXT NOT NULL,
    checked_in_at TEXT NOT NULL,
    started_at TEXT,
    ended_at TEXT,
    cancel_reason TEXT,
    status TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_visits_booking_id" ON "clinics_visits" (booking_id);
CREATE INDEX "clinics_visits_patient_party_id" ON "clinics_visits" (patient_party_id);
CREATE INDEX "clinics_visits_status" ON "clinics_visits" (status);
