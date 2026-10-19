-- Unallocating a payment from an open invoice (#70, part 2). An allocation
-- that is undone is marked removed, never deleted; NULL while it is active.
ALTER TABLE "wl_invoicing_allocations" ADD COLUMN removed_at TEXT;
-- Why: `payment_reversed`, `allocation_moved` or `recovery_reversed`.
ALTER TABLE "wl_invoicing_allocations" ADD COLUMN removed_reason TEXT;
-- The idempotency key of the unallocation that removed it.
ALTER TABLE "wl_invoicing_allocations" ADD COLUMN removed_key TEXT;

CREATE INDEX "wl_invoicing_allocations_active"
    ON "wl_invoicing_allocations" (invoice_id, source_type, source_id)
    WHERE removed_at IS NULL;

-- One row per unallocation run, by idempotency key: `done`, or `failed`
-- with the refusal's message. No foreign key: a refusal for an unknown
-- invoice is logged too.
CREATE TABLE "wl_invoicing_unallocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    -- The idempotency key, e.g. `payments.allocation:<id>`.
    key TEXT NOT NULL,
    reason TEXT NOT NULL,
    -- The payment's reversal reason key (`bank_return`, ...).
    reversal_reason TEXT,
    -- `done` or `failed`.
    outcome TEXT NOT NULL,
    -- The refusal's message; NULL when done.
    message TEXT,
    -- The transition that ran; NULL when failed.
    transition TEXT,
    -- The invoice's balance after; NULL when failed.
    balance_after INTEGER,
    ran_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_unallocations_key"
    ON "wl_invoicing_unallocations" (key);
CREATE INDEX "wl_invoicing_unallocations_invoice"
    ON "wl_invoicing_unallocations" (invoice_id);
