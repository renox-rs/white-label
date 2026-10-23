-- 08 Auto vehicle register (`AUT.VEH.01`, 08 section 6 *Vehicle*): a customer's
-- vehicle with its UAE plate (emirate, code, number), VIN, make, model, year,
-- odometer and owner or fleet. `fleet_contract_id` names the fleet contract
-- (built with the fleet contracts) the vehicle is serviced under.
CREATE TABLE "auto_vehicles" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plate_emirate TEXT NOT NULL,
    plate_code TEXT NOT NULL,
    plate_number TEXT NOT NULL,
    vin TEXT,
    make TEXT NOT NULL,
    model_name TEXT NOT NULL,
    model_year INTEGER,
    engine TEXT,
    colour TEXT,
    owner_party_id INTEGER NOT NULL,
    fleet_contract_id INTEGER,
    odometer_km INTEGER NOT NULL DEFAULT 0,
    odometer_at TEXT,
    next_service_date TEXT,
    next_service_odometer INTEGER,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_vehicles_plate_emirate_plate_code_plate_number" ON "auto_vehicles" (plate_emirate, plate_code, plate_number);
CREATE UNIQUE INDEX "auto_vehicles_vin" ON "auto_vehicles" (vin);
CREATE INDEX "auto_vehicles_owner_party_id" ON "auto_vehicles" (owner_party_id);
CREATE INDEX "auto_vehicles_fleet_contract_id" ON "auto_vehicles" (fleet_contract_id);
