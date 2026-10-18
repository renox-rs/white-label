CREATE TABLE "wl_invoicing_vat_summaries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The legal entity the summary is for.
    legal_entity_id INTEGER NOT NULL,
    -- The tax period, first and last day (inclusive).
    period_start TEXT NOT NULL,
    period_end TEXT NOT NULL,
    -- open, prepared, reviewed or locked.
    status TEXT NOT NULL DEFAULT 'open',
    prepared_by INTEGER,
    prepared_at TEXT,
    reviewed_by INTEGER,
    reviewed_at TEXT,
    locked_by INTEGER,
    locked_at TEXT,
    -- Why the reviewer sent it back to `prepared`.
    send_back_reason TEXT,
    -- The filing reference, given at `lock`.
    filing_reference TEXT,
    created_at TEXT,
    updated_at TEXT
);

-- One summary per entity and period.
CREATE UNIQUE INDEX "wl_invoicing_vat_summaries_entity_period"
    ON "wl_invoicing_vat_summaries" (legal_entity_id, period_start);
CREATE INDEX "wl_invoicing_vat_summaries_status" ON "wl_invoicing_vat_summaries" (status);
