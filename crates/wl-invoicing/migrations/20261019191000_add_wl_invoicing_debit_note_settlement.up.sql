-- What the settle and cancel of a supplier debit note record (#77): the
-- supplier's tax credit note, the settlement and the cancellation, and the
-- ledger of what was set off or refunded.
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN tax_credit_note_number TEXT;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN tax_credit_note_date TEXT;
-- The file of the supplier's tax credit note (a document of wl-documents).
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN tax_credit_note_document INTEGER;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN tax_credit_note_by INTEGER;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN settled_at TEXT;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN settled_by INTEGER;
-- What the supplier paid short, accepted with a reason; 0 otherwise.
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN settle_difference INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN settle_difference_reason TEXT;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN cancelled_at TEXT;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN cancelled_by INTEGER;
ALTER TABLE "wl_invoicing_supplier_debit_notes" ADD COLUMN cancel_reason TEXT;

-- Money that left the debit balance: set off on a later bill, refunded by the
-- supplier, or the accepted difference. Rows are never deleted.
CREATE TABLE "wl_invoicing_supplier_debit_note_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    debit_note_id INTEGER NOT NULL
        REFERENCES "wl_invoicing_supplier_debit_notes" (id) ON DELETE CASCADE,
    -- `set_off`, `refund` or `difference`.
    kind TEXT NOT NULL,
    -- The bill it was set off on, or the payment that refunded it (a subject).
    target_type TEXT,
    target_id INTEGER,
    amount INTEGER NOT NULL,
    -- The idempotency key of a refund (the payment's allocation); unique.
    key TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_supplier_debit_note_movements_note"
    ON "wl_invoicing_supplier_debit_note_movements" (debit_note_id);
CREATE UNIQUE INDEX "wl_invoicing_supplier_debit_note_movements_key"
    ON "wl_invoicing_supplier_debit_note_movements" (key);
