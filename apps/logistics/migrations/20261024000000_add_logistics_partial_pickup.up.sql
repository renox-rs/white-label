-- The weight and price a shipment is charged on, and the client's weight
-- bands (03 Logistics §6 *Shipment*, §4.1; BL-LOG-03). Weights are grams,
-- money is fils. The bands stand in for the rate card's zone prices
-- (`FIN.RAT.01`) until wl-invoicing has them.
ALTER TABLE "logistics_shipments" ADD COLUMN weight_grams INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_shipments" ADD COLUMN price INTEGER NOT NULL DEFAULT 0;
UPDATE "logistics_shipments" SET weight_grams = booked_weight_grams;

CREATE TABLE "logistics_weight_bands" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- The zone (the shipment's area); blank is every zone.
    zone TEXT NOT NULL DEFAULT '',
    -- The band covers weights up to this many grams.
    up_to_grams INTEGER NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_weight_bands_band" ON "logistics_weight_bands" (client_party_id, zone, up_to_grams);
