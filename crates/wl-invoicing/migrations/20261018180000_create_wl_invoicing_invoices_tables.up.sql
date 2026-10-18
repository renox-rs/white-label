CREATE TABLE "wl_invoicing_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- From the `invoice` series at `issue`; NULL while a draft.
    number TEXT UNIQUE,
    kind TEXT NOT NULL DEFAULT 'tax_invoice',
    legal_entity_id INTEGER,
    -- JSON `IssuerSnapshot`, taken at `issue` and never updated.
    issuer_snapshot TEXT,
    party_id INTEGER NOT NULL,
    -- JSON `PartySnapshot` (the billing identity), taken at `issue`.
    party_snapshot TEXT,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    scope_type TEXT,
    scope_id INTEGER,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The rate to AED × 1,000,000; NULL for a foreign currency until set.
    exchange_rate_micro INTEGER,
    issue_date TEXT,
    date_of_supply TEXT,
    due_date TEXT,
    -- Money in fils of the invoice's currency.
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- The VAT in AED fils; NULL while the rate is unknown.
    vat_total_aed INTEGER,
    allocated_total INTEGER NOT NULL DEFAULT 0,
    balance INTEGER NOT NULL DEFAULT 0,
    owner_id INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    disputed INTEGER NOT NULL DEFAULT 0,
    write_off_amount INTEGER,
    write_off_reason_code TEXT,
    write_off_reason_text TEXT,
    write_off_approval_id INTEGER,
    write_off_date TEXT,
    write_off_notice_on TEXT,
    write_off_recovered_amount INTEGER NOT NULL DEFAULT 0,
    write_off_export_pending INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_invoices_status" ON "wl_invoicing_invoices" (status);
CREATE INDEX "wl_invoicing_invoices_party" ON "wl_invoicing_invoices" (party_id);
CREATE INDEX "wl_invoicing_invoices_source" ON "wl_invoicing_invoices" (source_type, source_id);

CREATE TABLE "wl_invoicing_invoice_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    description TEXT NOT NULL,
    -- Thousandths: 1000 = 1.
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    unit_price INTEGER NOT NULL,
    discount INTEGER NOT NULL DEFAULT 0,
    net_amount INTEGER NOT NULL,
    vat_category TEXT NOT NULL,
    vat_rate_bp INTEGER NOT NULL DEFAULT 0,
    vat_amount INTEGER NOT NULL DEFAULT 0,
    -- The line VAT in AED fils; NULL while the rate is unknown.
    vat_amount_aed INTEGER,
    gross_amount INTEGER NOT NULL,
    origin_type TEXT,
    origin_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_invoice_lines_position"
    ON "wl_invoicing_invoice_lines" (invoice_id, position);

CREATE TABLE "wl_invoicing_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    -- A payment or a credit note.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    allocated_at TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_allocations_invoice" ON "wl_invoicing_allocations" (invoice_id);
CREATE INDEX "wl_invoicing_allocations_source" ON "wl_invoicing_allocations" (source_type, source_id);
