-- Lead sources (wl-sales §3, §7): where a lead or a campaign's customers
-- come from. The nine defaults are seeded idempotently and the client changes
-- them afterwards; a source in use is deactivated, never deleted.
CREATE TABLE "wl_sales_lead_sources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    active INTEGER NOT NULL DEFAULT 1,
    seeded INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);
