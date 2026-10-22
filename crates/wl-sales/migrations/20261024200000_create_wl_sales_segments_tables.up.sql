-- Customer segments and the broadcasts of retention campaigns (wl-sales §3
-- *Customer segment*, *Campaign*, §5.10; BL-SAL-25). A segment keeps its
-- rules, never a list of members: the members are worked out each time the
-- segment is used.
CREATE TABLE "wl_sales_segments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- `all` or `any`: how the rules combine.
    match_mode TEXT NOT NULL DEFAULT 'all',
    -- The rules as JSON, e.g. `[{"kind":"no_sale_for_days","days":60}]`.
    rules TEXT NOT NULL DEFAULT '[]',
    -- The member count of the last preview, an estimate.
    last_member_count INTEGER,
    last_counted_at TEXT,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_segments_name" ON "wl_sales_segments" (name);

-- Each time a retention campaign asked wl-notify for a broadcast: the
-- request, and how it ended. A campaign has at most one completed broadcast
-- and at most one open request.
CREATE TABLE "wl_sales_campaign_broadcasts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    campaign_id INTEGER NOT NULL REFERENCES "wl_sales_campaigns" (id),
    -- The wl-notify broadcast; empty when wl-notify refused the request.
    broadcast_id INTEGER,
    -- The user named as the requester (`NTF.ACL.04`).
    requested_by INTEGER NOT NULL,
    -- `requested`, `completed`, `cancelled` or `failed`.
    status TEXT NOT NULL DEFAULT 'requested',
    -- Why it did not go: the refusal, or the reason it was cancelled.
    reason TEXT,
    send_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_campaign_broadcasts_campaign" ON "wl_sales_campaign_broadcasts" (campaign_id);
CREATE INDEX "wl_sales_campaign_broadcasts_broadcast" ON "wl_sales_campaign_broadcasts" (broadcast_id);
CREATE UNIQUE INDEX "wl_sales_campaign_broadcasts_one_completed" ON "wl_sales_campaign_broadcasts" (campaign_id) WHERE status = 'completed';
CREATE UNIQUE INDEX "wl_sales_campaign_broadcasts_one_open" ON "wl_sales_campaign_broadcasts" (campaign_id) WHERE status = 'requested';
