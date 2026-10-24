-- 08 Auto: test drives and vehicle incidents (`AUT.ACL.22`, BL-AUT-06). A test
-- drive records the driver, the times and the mileage out and in; an incident
-- links the ticket of the category `vehicle_incident` to the job card and keeps
-- the branch manager's liability decision; the repair of damage the workshop
-- caused is an internal job card (the workshop pays, nothing is invoiced).
ALTER TABLE "auto_job_cards" ADD COLUMN internal BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_cards" ADD COLUMN incident_id INTEGER;

CREATE TABLE "auto_test_drives" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    driver_user_id INTEGER NOT NULL,
    out_at TEXT NOT NULL,
    odometer_out INTEGER NOT NULL,
    in_at TEXT,
    odometer_in INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_test_drives_job_card_id" ON "auto_test_drives" (job_card_id);

CREATE TABLE "auto_incidents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    vehicle_id INTEGER NOT NULL,
    ticket_id INTEGER,
    ticket_number TEXT,
    description TEXT NOT NULL,
    road_accident BOOLEAN NOT NULL DEFAULT FALSE,
    reported_by INTEGER NOT NULL,
    liability TEXT,
    liability_reason TEXT,
    decided_by INTEGER,
    decided_at TEXT,
    internal_job_card_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_incidents_job_card_id" ON "auto_incidents" (job_card_id);
