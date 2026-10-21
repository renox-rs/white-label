-- A vehicle taken off the road for an expired document (03 Logistics §7.2
-- `LOG.RUL.10`; BL-LOG-25).
CREATE TABLE "logistics_vehicle_groundings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vehicle_asset_id INTEGER NOT NULL,
    document_kind TEXT NOT NULL,
    expired_on TEXT NOT NULL,
    grounded_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_vehicle_groundings_once" ON "logistics_vehicle_groundings" (vehicle_asset_id, document_kind, expired_on);
CREATE INDEX "logistics_vehicle_groundings_vehicle" ON "logistics_vehicle_groundings" (vehicle_asset_id);
