-- A change of the loyalty terms with its effective date (wl-sales §4.7,
-- §7 *Notice before a programme change takes effect*; BL-SAL-26). Sales from
-- the effective date earn on the new terms; sales before it, and the lots
-- already earned, keep the old ones.
CREATE TABLE "wl_sales_loyalty_terms_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    programme_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_programmes" (id),
    effective_from TEXT NOT NULL,
    -- What changes: empty keeps the programme's value.
    earn_rate INTEGER,
    minimum_amount INTEGER,
    excluded_items TEXT,
    validity_months INTEGER,
    terms_version TEXT,
    terms_text_en TEXT,
    terms_text_ar TEXT,
    created_by INTEGER,
    -- The members told when the change was saved.
    notified_members INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_loyalty_terms_changes_programme" ON "wl_sales_loyalty_terms_changes" (programme_id, effective_from);
