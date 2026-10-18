CREATE TABLE "wl_invoicing_vat_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The VAT summary it is carried into; NULL while `pending`.
    summary_id INTEGER,
    -- The issuing legal entity of the source document.
    legal_entity_id INTEGER,
    -- late_document, earlier_period_credit_note, bad_debt_relief,
    -- bad_debt_recovery, supplier_debit_note or manual.
    kind TEXT NOT NULL,
    -- The source document, e.g. `invoicing.invoice:42`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- The VAT amount, in AED fils.
    amount INTEGER NOT NULL,
    reason TEXT,
    -- pending, proposed, included or excluded.
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_vat_adjustments_source"
    ON "wl_invoicing_vat_adjustments" (source_type, source_id);
CREATE INDEX "wl_invoicing_vat_adjustments_status" ON "wl_invoicing_vat_adjustments" (status);
CREATE INDEX "wl_invoicing_vat_adjustments_summary" ON "wl_invoicing_vat_adjustments" (summary_id);

-- One bad-debt relief per invoice (§5.9 step 3). Only that kind: a source
-- may get several recovery adjustments, one per recovery.
CREATE UNIQUE INDEX "wl_invoicing_vat_adjustments_one_relief"
    ON "wl_invoicing_vat_adjustments" (kind, source_type, source_id)
    WHERE kind = 'bad_debt_relief';
