-- Compliance flags of a building and expired compliance certificates (01
-- Property §4.5, §10; BL-PRP-25): the flags wl-ticketing raises on a building
-- (`visit_missed`, `no_contract`, `vendor_document_expired`) and the flag
-- `certificate_expired` raised the day after a compliance certificate
-- expires. A flag is open until it is cleared; it never blocks leasing.
CREATE TABLE "property_building_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    building_id INTEGER NOT NULL REFERENCES "property_buildings" (id),
    cause TEXT NOT NULL,
    -- The compliance-critical ticket category, or the certificate kind.
    category TEXT NOT NULL DEFAULT '',
    raised_on TEXT NOT NULL,
    cleared_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_building_flags_building" ON "property_building_flags" (building_id, cleared_on);
