-- The buildings a service contract covers (01 Property §4.4 step 1, `PRP.RUL.05`,
-- BL-PRP-34): recorded when the contract becomes active, so an approved AMC bill
-- is spread over the buildings and their owners.
CREATE TABLE "property_contract_buildings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL,
    building_id INTEGER NOT NULL REFERENCES "property_buildings" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "property_contract_buildings_contract_building" ON "property_contract_buildings" (contract_id, building_id);
