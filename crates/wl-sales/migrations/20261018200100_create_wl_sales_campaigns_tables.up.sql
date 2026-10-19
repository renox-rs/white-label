-- Campaigns (wl-sales §3, §4.6), the codes of deleted campaigns and the
-- promo codes a campaign carries (SAL.DSC.01 gets only this part for now).
-- The status moves only through the Campaign machine.
CREATE TABLE "wl_sales_campaigns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL DEFAULT '',
    -- Trimmed, upper case; unique over every state.
    code TEXT NOT NULL UNIQUE,
    purpose TEXT,
    marketing_channel TEXT,
    lead_source_id INTEGER REFERENCES "wl_sales_lead_sources" (id),
    brand TEXT,
    legal_entity_id INTEGER,
    -- The branch scope (a Subject); both empty for global.
    scope_type TEXT,
    scope_id TEXT,
    owner_id INTEGER,
    start_date TEXT,
    end_date TEXT,
    -- AED fils; empty means no over-budget check and no alerts.
    budget INTEGER,
    promo_code TEXT,
    -- Retention only.
    segment_id INTEGER,
    template_key TEXT,
    message_channel TEXT,
    send_at TEXT,
    retention_set_by INTEGER,
    lock_version INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_campaigns_status" ON "wl_sales_campaigns" (status, start_date);
CREATE INDEX "wl_sales_campaigns_lead_source" ON "wl_sales_campaigns" (lead_source_id);

-- The code of a deleted campaign stays taken.
CREATE TABLE "wl_sales_campaign_codes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    campaign_id INTEGER NOT NULL,
    deleted_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_sales_promo_codes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    campaign_id INTEGER NOT NULL UNIQUE REFERENCES "wl_sales_campaigns" (id),
    uses INTEGER NOT NULL DEFAULT 0,
    max_uses INTEGER,
    valid_to TEXT,
    created_at TEXT,
    updated_at TEXT
);
