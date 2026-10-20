-- The portfolio (01 Property §6, PRP.PRT.01, PRP.PRT.02): buildings, their
-- floors and their units. A unit's status moves only through the Unit machine.
CREATE TABLE "property_buildings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Short code, unique, e.g. `TWR-A`.
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL DEFAULT '',
    name_ar TEXT,
    emirate TEXT NOT NULL DEFAULT 'dubai',
    community TEXT,
    plot_number TEXT,
    -- DEWA premise number (Abu Dhabi: the ADDC account).
    premise_number TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "property_floors" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    building_id INTEGER NOT NULL REFERENCES "property_buildings" (id) ON DELETE CASCADE,
    -- 0 = ground, negative = basement.
    level INTEGER NOT NULL DEFAULT 0,
    name TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (building_id, level)
);

CREATE TABLE "property_units" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    building_id INTEGER NOT NULL REFERENCES "property_buildings" (id) ON DELETE CASCADE,
    floor_id INTEGER REFERENCES "property_floors" (id) ON DELETE SET NULL,
    number TEXT NOT NULL,
    unit_type TEXT NOT NULL DEFAULT 'apartment',
    usage TEXT NOT NULL DEFAULT 'residential',
    -- Square feet in thousandths.
    area_sqft_milli INTEGER,
    bedrooms INTEGER,
    -- AED fils per year.
    asking_rent INTEGER,
    notes TEXT,
    status TEXT NOT NULL DEFAULT 'available',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (building_id, number)
);

CREATE INDEX "property_units_building_status" ON "property_units" (building_id, status);
