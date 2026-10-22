-- 05 Clinics `CLN.RUL.01` (BL-CLN-12): who was told that a practitioner's
-- licence or branch privileges expired, once per person and expiry, so that a
-- run that is tried again sends nothing twice.
CREATE TABLE "clinics_licence_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    expires_on TEXT NOT NULL,
    recipient_user_id INTEGER NOT NULL,
    appointments INTEGER NOT NULL DEFAULT 0,
    sent_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_licence_alerts_expiry" ON "clinics_licence_alerts" (employee_id, kind, expires_on, recipient_user_id);
