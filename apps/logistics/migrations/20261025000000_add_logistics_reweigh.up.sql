-- The re-weigh at the hub (03 Logistics §4.1 *Weight or size differs at the
-- hub*, `LOG.HUB.01`; BL-LOG-04): the client's tolerance and volumetric
-- divisor (a stand-in for the rate card's, `FIN.RAT.01`), every measurement,
-- and the marks on the shipment and on its surcharge line. Weights are
-- grams, money is fils.
ALTER TABLE "logistics_shipments" ADD COLUMN measured_weight_grams INTEGER;
ALTER TABLE "logistics_shipments" ADD COLUMN reweigh_price INTEGER;
ALTER TABLE "logistics_shipments" ADD COLUMN invoiced_on TEXT;
ALTER TABLE "logistics_charges" ADD COLUMN held INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_charges" ADD COLUMN ticket_id INTEGER;
ALTER TABLE "logistics_charges" ADD COLUMN billed_on TEXT;

CREATE TABLE "logistics_client_terms" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- The accepted difference between booked and measured weight; 0 when
    -- the rate card sets none.
    tolerance_grams INTEGER NOT NULL DEFAULT 0,
    -- Length x width x height in cm over this, times 1000, is the
    -- volumetric weight in grams; none when the card does not price by
    -- volume.
    volumetric_divisor INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_client_terms_client" ON "logistics_client_terms" (client_party_id);

CREATE TABLE "logistics_measurements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    weight_grams INTEGER NOT NULL,
    length_cm INTEGER NOT NULL DEFAULT 0,
    width_cm INTEGER NOT NULL DEFAULT 0,
    height_cm INTEGER NOT NULL DEFAULT 0,
    -- The higher of the actual and the volumetric weight.
    chargeable_grams INTEGER NOT NULL,
    -- The photo of the scale.
    photo_ref TEXT,
    measured_by INTEGER,
    measured_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_measurements_shipment" ON "logistics_measurements" (shipment_id);
