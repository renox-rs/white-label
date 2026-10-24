-- Marketing channels (wl-sales §3 Campaign, BL-SAL-28): where a campaign runs,
-- a table like the lead sources instead of a fixed list. The module's twelve
-- channels are the seed; a vertical adds its own and deactivates the others.
-- A campaign keeps its channel: wl_sales_campaigns.marketing_channel already
-- holds the stored name of the old list, which is the key of the seeded row.
CREATE TABLE "wl_sales_marketing_channels" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    active INTEGER NOT NULL DEFAULT 1,
    seeded INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

INSERT INTO "wl_sales_marketing_channels" (code, name_en, name_ar, active, seeded) VALUES
    ('google', 'Google', '', 1, 1),
    ('meta', 'Meta', '', 1, 1),
    ('tiktok', 'TikTok', '', 1, 1),
    ('other_online', 'Other online', '', 1, 1),
    ('print', 'Print', '', 1, 1),
    ('radio_outdoor', 'Radio or outdoor', '', 1, 1),
    ('event', 'Event', '', 1, 1),
    ('email', 'E-mail', '', 1, 1),
    ('sms', 'SMS', '', 1, 1),
    ('whatsapp', 'WhatsApp', '', 1, 1),
    ('partner', 'Partner', '', 1, 1),
    ('other', 'Other', '', 1, 1);

CREATE INDEX "wl_sales_campaigns_marketing_channel" ON "wl_sales_campaigns" (marketing_channel);
