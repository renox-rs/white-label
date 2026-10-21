-- Usage records (§3, §5.2): the priced hours (or other usage) waiting to be
-- billed, and where they were billed. An adjustment timesheet's extra hours
-- are a record of their own that names the adjustment.
CREATE TABLE "wl_invoicing_usage_records" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The usage's source, e.g. `attendance.timesheet:3`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The hours category of the source (`normal`, `overtime`...).
    category TEXT NOT NULL DEFAULT 'normal',
    description TEXT NOT NULL,
    -- Thousandths of an hour: 1000 = 1 h.
    quantity_milli INTEGER NOT NULL,
    -- The price of one unit after the multipliers of the rate card line.
    unit_price INTEGER NOT NULL,
    -- The rate date the price was taken at.
    rate_date TEXT,
    vat_category TEXT NOT NULL DEFAULT 'standard',
    -- An extra record of an adjustment: the record it corrects and the
    -- adjustment timesheet.
    adjusts_id INTEGER,
    adjustment_type TEXT,
    adjustment_id INTEGER,
    -- Where it was billed: the invoice and the line's position.
    invoice_id INTEGER,
    invoice_position INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_usage_records_party" ON "wl_invoicing_usage_records" (party_id);
CREATE INDEX "wl_invoicing_usage_records_source" ON "wl_invoicing_usage_records" (source_type, source_id);
CREATE INDEX "wl_invoicing_usage_records_invoice" ON "wl_invoicing_usage_records" (invoice_id);
