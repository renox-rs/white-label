-- The supplier debit note (#76, part 2): a note against a supplier bill, or
-- waiting for its bill, with its lines. The bill lines also get the PO line
-- they were ordered on, so a returned line can be matched to a bill line.
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN po_line_type TEXT;
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN po_line_id INTEGER;

CREATE INDEX "wl_invoicing_supplier_bill_lines_po_line"
    ON "wl_invoicing_supplier_bill_lines" (po_line_type, po_line_id);

CREATE TABLE "wl_invoicing_supplier_debit_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- From the `supplier_debit_note` series at `issue`; NULL while a draft.
    number TEXT,
    -- The receiving legal entity: the bill's, or the return's while unlinked.
    legal_entity_id INTEGER NOT NULL,
    -- The supplier (a party).
    party_id INTEGER NOT NULL,
    -- The bill it reduces; NULL while it waits for the bill.
    bill_id INTEGER REFERENCES "wl_invoicing_supplier_bills" (id),
    -- `return` or `manual`.
    origin TEXT NOT NULL DEFAULT 'manual',
    -- The return it was drafted from (a subject); one draft per return.
    source_type TEXT,
    source_id INTEGER,
    -- `price_difference`, `short_delivery`, `quality` or `other`; NULL for a
    -- return.
    reason_code TEXT,
    reason TEXT NOT NULL DEFAULT '',
    currency TEXT NOT NULL DEFAULT 'AED',
    exchange_rate_micro INTEGER,
    -- Money in fils of the note's currency.
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- The VAT in AED fils; NULL while the rate is unknown.
    vat_total_aed INTEGER,
    -- What reduced the bill, and what is open as the supplier's debit
    -- balance (the two add up to the gross once issued).
    applied_to_bill INTEGER NOT NULL DEFAULT 0,
    debit_balance INTEGER NOT NULL DEFAULT 0,
    issue_date TEXT,
    created_by INTEGER,
    issued_by INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_supplier_debit_notes_number"
    ON "wl_invoicing_supplier_debit_notes" (number);
CREATE UNIQUE INDEX "wl_invoicing_supplier_debit_notes_source"
    ON "wl_invoicing_supplier_debit_notes" (source_type, source_id);
CREATE INDEX "wl_invoicing_supplier_debit_notes_status"
    ON "wl_invoicing_supplier_debit_notes" (status);
CREATE INDEX "wl_invoicing_supplier_debit_notes_bill"
    ON "wl_invoicing_supplier_debit_notes" (bill_id);
CREATE INDEX "wl_invoicing_supplier_debit_notes_party"
    ON "wl_invoicing_supplier_debit_notes" (party_id);

CREATE TABLE "wl_invoicing_supplier_debit_note_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    debit_note_id INTEGER NOT NULL
        REFERENCES "wl_invoicing_supplier_debit_notes" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    -- The bill line it reduces; NULL while the note waits for its bill.
    bill_line_id INTEGER,
    -- The PO line the goods were ordered on (a subject), if known.
    po_line_type TEXT,
    po_line_id INTEGER,
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
    -- The cost object the cost is reversed on, if any (a subject).
    cost_type TEXT,
    cost_id INTEGER,
    -- Whose price the line is at: `po` or `bill`.
    priced_at TEXT NOT NULL DEFAULT 'bill',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_supplier_debit_note_lines_position"
    ON "wl_invoicing_supplier_debit_note_lines" (debit_note_id, position);
CREATE INDEX "wl_invoicing_supplier_debit_note_lines_bill_line"
    ON "wl_invoicing_supplier_debit_note_lines" (bill_line_id);
