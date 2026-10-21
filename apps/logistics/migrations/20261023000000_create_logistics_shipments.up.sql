-- The Shipment, its parcels, tracking events, charge lines and the client's
-- prices (03 Logistics §6, §7.1 *Shipment*; BL-LOG-01, BL-LOG-02).
-- Money is fils; weights are grams.
CREATE TABLE "logistics_shipments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `SHP-0007`, kept when the shipment is voided.
    number TEXT NOT NULL,
    client_party_id INTEGER NOT NULL,
    -- The shipper's own reference; unique per client across all states.
    client_reference TEXT,
    booked_by_user_id INTEGER,
    recipient_name TEXT NOT NULL DEFAULT '',
    recipient_phone TEXT NOT NULL DEFAULT '',
    address TEXT NOT NULL DEFAULT '',
    area TEXT NOT NULL DEFAULT '',
    cod_amount INTEGER NOT NULL DEFAULT 0,
    booked_weight_grams INTEGER NOT NULL DEFAULT 0,
    parcel_count INTEGER NOT NULL DEFAULT 0,
    -- Parcels scanned at pickup or at a hub (`void` needs none).
    parcels_scanned INTEGER NOT NULL DEFAULT 0,
    -- The wl-delivery delivery of the shipment.
    delivery_id INTEGER,
    -- The client's instruction to send it back (`LOG.ACL.09`).
    return_instruction TEXT,
    return_instruction_by INTEGER,
    return_instruction_at TEXT,
    -- The client's e-POD of the returned goods.
    return_pod_ref TEXT,
    status TEXT NOT NULL DEFAULT 'created',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_shipments_number" ON "logistics_shipments" (number);
CREATE UNIQUE INDEX "logistics_shipments_client_reference" ON "logistics_shipments" (client_party_id, client_reference);
CREATE INDEX "logistics_shipments_status" ON "logistics_shipments" (status);
CREATE INDEX "logistics_shipments_delivery" ON "logistics_shipments" (delivery_id);

CREATE TABLE "logistics_parcels" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    barcode TEXT NOT NULL,
    weight_grams INTEGER NOT NULL DEFAULT 0,
    -- `booked`, `scanned` or `not_received`.
    state TEXT NOT NULL DEFAULT 'booked',
    scanned_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_parcels_barcode" ON "logistics_parcels" (barcode);
CREATE INDEX "logistics_parcels_shipment" ON "logistics_parcels" (shipment_id);

CREATE TABLE "logistics_tracking_events" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    kind TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    occurred_at TEXT NOT NULL,
    actor_user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_tracking_events_shipment" ON "logistics_tracking_events" (shipment_id);

CREATE TABLE "logistics_charges" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    client_party_id INTEGER NOT NULL,
    -- `cancellation` or `return`.
    kind TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    recorded_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

-- One cancellation fee and one return charge per shipment, whichever path.
CREATE UNIQUE INDEX "logistics_charges_kind" ON "logistics_charges" (shipment_id, kind);

CREATE TABLE "logistics_client_rates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- `cancellation` or `return`: the lines of the rate card these two
    -- issues read until the rate cards of wl-invoicing exist.
    kind TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_client_rates_kind" ON "logistics_client_rates" (client_party_id, kind);
