-- 08 Auto fleet service plan (`AUT.FLT.01`, `WRK.PPM.01`, `AUT.RUL.04`): a
-- service that falls due for a vehicle under a fleet contract, by mileage
-- and/or months. When it is due it is booked through wl-scheduling
-- (`booking_id`); a delivered job card moves it on.
CREATE TABLE "auto_fleet_services" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL REFERENCES "auto_fleet_contracts" (id),
    vehicle_id INTEGER NOT NULL REFERENCES "auto_vehicles" (id),
    service_code TEXT NOT NULL,
    interval_km INTEGER,
    interval_months INTEGER,
    last_service_date TEXT,
    last_service_km INTEGER,
    next_due_date TEXT,
    next_due_km INTEGER,
    booking_id INTEGER,
    requested_for_date TEXT,
    requested_at TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_fleet_services_contract_id" ON "auto_fleet_services" (contract_id);
CREATE INDEX "auto_fleet_services_vehicle_id" ON "auto_fleet_services" (vehicle_id);
CREATE INDEX "auto_fleet_services_booking_id" ON "auto_fleet_services" (booking_id);
