CREATE TABLE "wl_invoicing_credit_note_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    credit_note_id INTEGER NOT NULL REFERENCES "wl_invoicing_credit_notes" (id),
    -- invoice_allocated, invoice_credited, refund_taken, refund_restored or
    -- allocation_restored.
    kind TEXT NOT NULL,
    -- What the amount went to or came back from: an allocation, an invoice
    -- or a refund; NULL for a refund not yet created.
    target_type TEXT,
    target_id INTEGER,
    -- Fils of the credit note's currency; always > 0.
    amount INTEGER NOT NULL CHECK (amount > 0),
    -- The refund attempt number; NULL for the first take.
    attempt INTEGER,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_credit_note_movements_credit_note"
    ON "wl_invoicing_credit_note_movements" (credit_note_id);
CREATE INDEX "wl_invoicing_credit_note_movements_target"
    ON "wl_invoicing_credit_note_movements" (target_type, target_id);
-- Every take and restore happens once. A NULL is never equal to another in
-- a unique index, so the optional parts count as empty or 0.
CREATE UNIQUE INDEX "wl_invoicing_credit_note_movements_key"
    ON "wl_invoicing_credit_note_movements" (
        credit_note_id,
        kind,
        COALESCE(target_type, ''),
        COALESCE(target_id, 0),
        COALESCE(attempt, 0)
    );
