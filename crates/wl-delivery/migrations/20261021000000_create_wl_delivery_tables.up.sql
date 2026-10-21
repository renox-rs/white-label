-- Runs, deliveries and their lines (wl-delivery §3, §4). A run is one
-- driver's trip; a delivery is one drop (or pickup) for a source record, on
-- at most one run. Statuses change only through the machines.

CREATE TABLE "wl_delivery_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    run_date TEXT NOT NULL,
    -- The depot the run leaves from, as text until the hubs exist (03).
    depot TEXT NOT NULL DEFAULT '',
    -- The driver: a user with the driver role.
    driver_user_id INTEGER NOT NULL,
    vehicle_type TEXT NOT NULL DEFAULT 'van',
    -- The vehicle: an asset of wl-inventory; NULL for an on-foot rider.
    vehicle_asset_id INTEGER,
    estimated_minutes INTEGER,
    planned_start TEXT,
    abort_reason TEXT,
    status TEXT NOT NULL DEFAULT 'planned',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_runs_date" ON "wl_delivery_runs" (run_date);
CREATE INDEX "wl_delivery_runs_driver" ON "wl_delivery_runs" (driver_user_id, run_date);
CREATE INDEX "wl_delivery_runs_status" ON "wl_delivery_runs" (status);

CREATE TABLE "wl_delivery_deliveries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL UNIQUE,
    direction TEXT NOT NULL DEFAULT 'delivery',
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    recipient_party_id INTEGER,
    recipient_name TEXT NOT NULL DEFAULT '',
    recipient_phone TEXT NOT NULL DEFAULT '',
    address TEXT NOT NULL DEFAULT '',
    area TEXT NOT NULL DEFAULT '',
    latitude_micro INTEGER,
    longitude_micro INTEGER,
    window_from TEXT,
    window_to TEXT,
    instructions TEXT NOT NULL DEFAULT '',
    perishable INTEGER NOT NULL DEFAULT 0,
    -- Cash on delivery, in fils; 0 when nothing is collected.
    amount_to_collect INTEGER NOT NULL DEFAULT 0,
    cheque_expected INTEGER NOT NULL DEFAULT 0,
    -- The proof methods required, copied from the setting of the source type
    -- when the delivery is created: `photo,signature`.
    pod_methods TEXT NOT NULL DEFAULT 'photo,signature',
    -- Whether any one of them is enough (else every one is needed).
    pod_any INTEGER NOT NULL DEFAULT 0,
    -- Whether the source type accepts partly accepted deliveries, copied
    -- from the setting when the delivery is created.
    partial_acceptance INTEGER NOT NULL DEFAULT 1,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_id INTEGER REFERENCES "wl_delivery_runs" (id) ON DELETE SET NULL,
    run_sequence INTEGER,
    cancel_reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_deliveries_status" ON "wl_delivery_deliveries" (status);
CREATE INDEX "wl_delivery_deliveries_run" ON "wl_delivery_deliveries" (run_id);
CREATE INDEX "wl_delivery_deliveries_source" ON "wl_delivery_deliveries" (source_type, source_id);

CREATE TABLE "wl_delivery_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    source_line_type TEXT,
    source_line_id INTEGER,
    description TEXT NOT NULL DEFAULT '',
    -- Quantities are decimals in the line's unit, in thousandths.
    quantity_milli INTEGER NOT NULL,
    -- The amount to collect for the whole line, in fils, when the source
    -- gives line amounts.
    amount_to_collect INTEGER,
    delivered_milli INTEGER NOT NULL DEFAULT 0,
    refused_milli INTEGER NOT NULL DEFAULT 0,
    refusal_reason TEXT,
    refusal_photo_ref TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_lines_delivery" ON "wl_delivery_lines" (delivery_id);

-- Failure and refusal reasons: configurable lists (wl-delivery §3, §7).
CREATE TABLE "wl_delivery_failure_reasons" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    -- Whether a failed attempt for this reason counts against the maximum.
    counts INTEGER NOT NULL DEFAULT 1,
    -- Whether the delivery may be tried again after it.
    allows_reattempt INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_delivery_refusal_reasons" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

-- One row per visit (wl-delivery §3 *Attempt*) and its proof. A proof is
-- never edited after it is saved (`WRK.ACL.06`).
CREATE TABLE "wl_delivery_attempts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES "wl_delivery_runs" (id) ON DELETE SET NULL,
    number INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    failure_reason TEXT,
    counts INTEGER NOT NULL DEFAULT 1,
    occurred_at TEXT NOT NULL,
    latitude_micro INTEGER,
    longitude_micro INTEGER,
    actor_user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_attempts_delivery" ON "wl_delivery_attempts" (delivery_id);

CREATE TABLE "wl_delivery_proofs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    attempt_id INTEGER NOT NULL UNIQUE REFERENCES "wl_delivery_attempts" (id) ON DELETE CASCADE,
    -- The methods used: `photo,signature`.
    methods TEXT NOT NULL DEFAULT '',
    recipient_name TEXT NOT NULL DEFAULT '',
    photo_ref TEXT,
    signature_ref TEXT,
    otp_verified INTEGER NOT NULL DEFAULT 0,
    cash_expected INTEGER NOT NULL DEFAULT 0,
    cash_collected INTEGER NOT NULL DEFAULT 0,
    cheque_reference TEXT,
    notes TEXT NOT NULL DEFAULT '',
    reversed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_delivery_proofs_delivery" ON "wl_delivery_proofs" (delivery_id);
