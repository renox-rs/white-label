-- The outlet outage and the pause of its channels, and the paper orders entered
-- afterwards as late entries (06 F&B §4.6 row *Internet or power outage at the
-- outlet*, §7.2 `FNB.RUL.15`; BL-FNB-14). Money is fils.
CREATE TABLE "fnb_outages" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    outlet_id INTEGER NOT NULL REFERENCES "fnb_outlets" (id),
    -- The device connection that went down (`devices`) or blank (`manual`).
    connection_key TEXT,
    source TEXT NOT NULL DEFAULT 'devices',
    status TEXT NOT NULL DEFAULT 'paused',
    started_at TEXT,
    paused_at TEXT,
    devices_back_at TEXT,
    resumed_at TEXT,
    resumed_by INTEGER,
    last_prompt_at TEXT,
    prompts INTEGER NOT NULL DEFAULT 0,
    reason TEXT,
    ticket_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_outages_outlet" ON "fnb_outages" (outlet_id);
CREATE INDEX "fnb_outages_status" ON "fnb_outages" (status);

-- A channel an outlet takes orders on: a brand on the ordering site, or an
-- aggregator's tablet.
CREATE TABLE "fnb_outlet_channels" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    outlet_id INTEGER NOT NULL REFERENCES "fnb_outlets" (id),
    -- `d2c` or `aggregator`.
    kind TEXT NOT NULL,
    brand_id INTEGER,
    channel TEXT NOT NULL,
    paused INTEGER NOT NULL DEFAULT 0,
    paused_at TEXT,
    paused_by INTEGER,
    outage_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_outlet_channels_outlet" ON "fnb_outlet_channels" (outlet_id);
CREATE INDEX "fnb_outlet_channels_brand" ON "fnb_outlet_channels" (brand_id);

-- An order written on paper during an outage and entered afterwards with its
-- business time and the late-entry flag.
CREATE TABLE "fnb_paper_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    outlet_id INTEGER NOT NULL REFERENCES "fnb_outlets" (id),
    channel TEXT NOT NULL,
    reference TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    method TEXT NOT NULL DEFAULT 'cash',
    note TEXT NOT NULL DEFAULT '',
    occurred_at TEXT,
    entered_at TEXT,
    entered_by INTEGER,
    business_date TEXT,
    late INTEGER NOT NULL DEFAULT 0,
    late_reason TEXT,
    late_entry_id INTEGER,
    outage_id INTEGER,
    reopened_closing_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_paper_orders_day" ON "fnb_paper_orders" (outlet_id, business_date);
