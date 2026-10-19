-- The money side of a supplier bill (#75, part 2): what is applied to it and
-- what is still open, in fils of the bill's currency.
ALTER TABLE "wl_invoicing_supplier_bills" ADD COLUMN allocated_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_invoicing_supplier_bills" ADD COLUMN balance INTEGER NOT NULL DEFAULT 0;
-- Nothing is applied to an existing bill, so all of the gross is open.
UPDATE "wl_invoicing_supplier_bills" SET balance = gross_total;

-- Money applied to a bill: an outgoing payment or a debit note. A removed
-- row is marked, never deleted; removed_at is NULL while it is active.
CREATE TABLE "wl_invoicing_supplier_bill_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_supplier_bills" (id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    allocated_at TEXT NOT NULL,
    created_by INTEGER,
    removed_at TEXT,
    -- Why: `payment_reversed` or `debit_note_cancelled`.
    removed_reason TEXT,
    -- The idempotency key of the unallocation that removed it.
    removed_key TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_supplier_bill_allocations_bill"
    ON "wl_invoicing_supplier_bill_allocations" (bill_id);
CREATE INDEX "wl_invoicing_supplier_bill_allocations_source"
    ON "wl_invoicing_supplier_bill_allocations" (source_type, source_id);

-- One row per bill unallocation run, by idempotency key: `done`, or
-- `failed` with the refusal's message. No foreign key: a refusal for an
-- unknown bill is logged too.
CREATE TABLE "wl_invoicing_bill_unallocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bill_id INTEGER NOT NULL,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    key TEXT NOT NULL,
    reason TEXT NOT NULL,
    -- The payment's reversal reason key (`bank_return`, ...).
    reversal_reason TEXT,
    -- `done` or `failed`.
    outcome TEXT NOT NULL,
    message TEXT,
    -- The transition that ran; NULL when failed.
    transition TEXT,
    -- The bill's balance after; NULL when failed.
    balance_after INTEGER,
    ran_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_bill_unallocations_key"
    ON "wl_invoicing_bill_unallocations" (key);
CREATE INDEX "wl_invoicing_bill_unallocations_bill"
    ON "wl_invoicing_bill_unallocations" (bill_id);
