-- 08 Auto job card (`AUT.JOB.01`, 08 section 6 *Job card*, 7.1 *Job card*): the
-- workshop work order for one visit of a vehicle. Check-in data (complaint,
-- odometer, fuel, intake condition), the technician, the estimate round the
-- customer answered, the QC checklist and the delivery. The workshop is the
-- scope (a wl-scheduling location) of the booking that was checked in.
CREATE TABLE "auto_job_cards" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vehicle_id INTEGER NOT NULL REFERENCES "auto_vehicles" (id),
    customer_party_id INTEGER NOT NULL,
    booking_id INTEGER,
    workshop_type TEXT,
    workshop_id INTEGER,
    advisor_user_id INTEGER,
    technician_user_id INTEGER,
    complaint TEXT NOT NULL DEFAULT '',
    odometer_in INTEGER NOT NULL DEFAULT 0,
    fuel_level TEXT,
    intake_condition TEXT,
    intake_by INTEGER,
    intake_recorded_at TEXT,
    intake_acknowledged_by TEXT,
    intake_signature TEXT,
    findings TEXT,
    accident_damage BOOLEAN NOT NULL DEFAULT FALSE,
    police_report_no TEXT,
    inspection_fee INTEGER NOT NULL DEFAULT 0,
    storage_fee_daily INTEGER NOT NULL DEFAULT 0,
    estimate_revision INTEGER NOT NULL DEFAULT 0,
    awaiting_since TEXT,
    first_approved_at TEXT,
    parts_wait_started_at TEXT,
    legal_supply_date TEXT,
    qc_checks TEXT NOT NULL DEFAULT '[]',
    qc_findings TEXT,
    qc_by INTEGER,
    ready_at TEXT,
    delivered_at TEXT,
    delivered_to TEXT,
    odometer_out INTEGER,
    next_service_date TEXT,
    next_service_odometer INTEGER,
    closed_reason TEXT,
    closed_at TEXT,
    invoice_id INTEGER,
    invoiced_at TEXT,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_job_cards_vehicle_id" ON "auto_job_cards" (vehicle_id);
CREATE INDEX "auto_job_cards_customer_party_id" ON "auto_job_cards" (customer_party_id);
CREATE UNIQUE INDEX "auto_job_cards_booking_id" ON "auto_job_cards" (booking_id);
CREATE INDEX "auto_job_cards_technician_user_id" ON "auto_job_cards" (technician_user_id);
CREATE INDEX "auto_job_cards_status" ON "auto_job_cards" (status);
