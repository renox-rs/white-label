-- The supplier bill base (#74, part 1): a bill from a supplier with its
-- lines, the reasons it is held, and a lock per supplier for the duplicate
-- check.
CREATE TABLE "wl_invoicing_supplier_bills" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The supplier (a party).
    party_id INTEGER NOT NULL,
    -- The receiving legal entity; the default entity when not given.
    legal_entity_id INTEGER,
    -- The supplier's invoice number as entered, and the same trimmed and
    -- lower-cased for the duplicate check.
    supplier_invoice_number TEXT NOT NULL,
    number_key TEXT NOT NULL,
    bill_date TEXT NOT NULL,
    due_date TEXT,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The rate to AED × 1,000,000 for a foreign currency; NULL for AED.
    exchange_rate_micro INTEGER,
    -- The service contract the bill is against, if any (a subject).
    contract_type TEXT,
    contract_id INTEGER,
    period_start TEXT,
    period_end TEXT,
    -- Money in fils of the bill's currency.
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- The gross in AED fils; NULL while the rate is unknown.
    gross_total_aed INTEGER,
    created_by INTEGER,
    approved_by INTEGER,
    approval_reason TEXT,
    void_reason TEXT,
    -- The earlier bill a held bill may duplicate.
    duplicate_of INTEGER REFERENCES "wl_invoicing_supplier_bills" (id),
    lock_version INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_supplier_bills_status" ON "wl_invoicing_supplier_bills" (status);
CREATE INDEX "wl_invoicing_supplier_bills_number"
    ON "wl_invoicing_supplier_bills" (party_id, number_key);
CREATE INDEX "wl_invoicing_supplier_bills_contract"
    ON "wl_invoicing_supplier_bills" (contract_type, contract_id);

CREATE TABLE "wl_invoicing_supplier_bill_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_supplier_bills" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    description TEXT NOT NULL,
    quantity_milli INTEGER NOT NULL,
    unit_price INTEGER NOT NULL,
    net_amount INTEGER NOT NULL,
    vat_category TEXT NOT NULL,
    vat_rate_bp INTEGER NOT NULL DEFAULT 0,
    vat_amount INTEGER NOT NULL DEFAULT 0,
    -- The line VAT and net in AED fils; NULL while the rate is unknown.
    vat_amount_aed INTEGER,
    net_amount_aed INTEGER,
    gross_amount INTEGER NOT NULL,
    service_date TEXT,
    -- The cost object the line is coded to, if any (a subject).
    cost_type TEXT,
    cost_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_supplier_bill_lines_position"
    ON "wl_invoicing_supplier_bill_lines" (bill_id, position);

-- One row per reason a bill is held, so every reason is shown.
CREATE TABLE "wl_invoicing_supplier_bill_exceptions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_supplier_bills" (id) ON DELETE CASCADE,
    -- `possible_duplicate`; other reasons are added by the matches.
    reason_code TEXT NOT NULL,
    related_bill_id INTEGER REFERENCES "wl_invoicing_supplier_bills" (id),
    created_at TEXT
);

CREATE INDEX "wl_invoicing_supplier_bill_exceptions_bill"
    ON "wl_invoicing_supplier_bill_exceptions" (bill_id);

-- Taken at `submit` so two bills of one supplier are checked one after the
-- other.
CREATE TABLE "wl_invoicing_supplier_locks" (
    party_id INTEGER PRIMARY KEY,
    n INTEGER NOT NULL DEFAULT 0
);
