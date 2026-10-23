-- The cold units, their temperature readings and the chef's decisions on a
-- temperature hold (06 F&B §4.6 row *A fridge or freezer reads out of range*,
-- §7.2 `FNB.RUL.05`; BL-FNB-12). Each unit is a stock location of the outlet:
-- a reading outside its range places a stock hold on the stock in it.
CREATE TABLE "fnb_cold_units" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    -- `fridge`, `freezer` or `hot`.
    kind TEXT NOT NULL,
    outlet_id INTEGER,
    -- The stock location (wl-inventory) of the unit: required.
    location_id INTEGER NOT NULL,
    -- The range in tenths of a degree.
    min_tenths INTEGER,
    max_tenths INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    -- The open stock hold, the equipment task and since when it is out of range.
    hold_id INTEGER,
    ticket_id INTEGER,
    ticket_number TEXT,
    out_since TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_cold_units_code" ON "fnb_cold_units" (code);
CREATE INDEX "fnb_cold_units_location" ON "fnb_cold_units" (location_id);

CREATE TABLE "fnb_temperature_readings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    unit_id INTEGER NOT NULL REFERENCES "fnb_cold_units" (id),
    tenths INTEGER NOT NULL,
    in_range INTEGER NOT NULL DEFAULT 1,
    note TEXT NOT NULL DEFAULT '',
    recorded_by INTEGER,
    recorded_at TEXT NOT NULL,
    -- The hold the reading placed or joined, and when the rule reacted.
    hold_id INTEGER,
    reacted_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_temperature_readings_unit" ON "fnb_temperature_readings" (unit_id);

-- What the chef recorded when deciding a hold line.
CREATE TABLE "fnb_hold_decisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    unit_id INTEGER NOT NULL REFERENCES "fnb_cold_units" (id),
    hold_id INTEGER NOT NULL,
    line_id INTEGER NOT NULL,
    decision TEXT NOT NULL,
    reading_tenths INTEGER NOT NULL,
    minutes_out INTEGER NOT NULL,
    decided_by INTEGER NOT NULL,
    waiting INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_hold_decisions_unit" ON "fnb_hold_decisions" (unit_id);
CREATE INDEX "fnb_hold_decisions_line" ON "fnb_hold_decisions" (line_id);
