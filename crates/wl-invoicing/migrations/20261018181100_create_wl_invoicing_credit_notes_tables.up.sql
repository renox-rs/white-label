CREATE TABLE "wl_invoicing_credit_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- From the `credit_note` series at `issue`; NULL while a draft.
    number TEXT UNIQUE,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id),
    -- Always the invoice's legal entity.
    legal_entity_id INTEGER,
    -- JSON `IssuerSnapshot`, taken at `issue` and never updated.
    issuer_snapshot TEXT,
    party_id INTEGER NOT NULL,
    -- JSON `PartySnapshot`, copied from the invoice at `issue`.
    party_snapshot TEXT,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The invoice's rate to AED × 1,000,000; NULL for AED or until set.
    exchange_rate_micro INTEGER,
    reason TEXT NOT NULL,
    issue_date TEXT,
    -- Money in fils of the credit note's currency.
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- The VAT in AED fils; NULL while the rate is unknown.
    vat_total_aed INTEGER,
    -- What is still unused; the gross total from `issue`, 0 for a draft.
    remaining INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL DEFAULT 'partial',
    release_origins INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_credit_notes_status" ON "wl_invoicing_credit_notes" (status);
CREATE INDEX "wl_invoicing_credit_notes_invoice" ON "wl_invoicing_credit_notes" (invoice_id);
CREATE INDEX "wl_invoicing_credit_notes_party" ON "wl_invoicing_credit_notes" (party_id);

CREATE TABLE "wl_invoicing_credit_note_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    credit_note_id INTEGER NOT NULL REFERENCES "wl_invoicing_credit_notes" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    invoice_line_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoice_lines" (id),
    description TEXT NOT NULL,
    net_amount INTEGER NOT NULL,
    -- The invoice line's category and rate.
    vat_category TEXT NOT NULL,
    vat_rate_bp INTEGER NOT NULL DEFAULT 0,
    vat_amount INTEGER NOT NULL DEFAULT 0,
    -- The line VAT in AED fils; NULL while the rate is unknown.
    vat_amount_aed INTEGER,
    gross_amount INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_credit_note_lines_position"
    ON "wl_invoicing_credit_note_lines" (credit_note_id, position);
CREATE INDEX "wl_invoicing_credit_note_lines_invoice_line"
    ON "wl_invoicing_credit_note_lines" (invoice_line_id);
