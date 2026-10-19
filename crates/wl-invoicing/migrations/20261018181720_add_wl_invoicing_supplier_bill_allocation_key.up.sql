-- The idempotency key of the payment that wrote an allocation (#75, part 3):
-- a second payment under the same key is skipped. NULL for a payment made
-- without a key; SQLite lets a unique index hold many NULLs.
ALTER TABLE "wl_invoicing_supplier_bill_allocations" ADD COLUMN key TEXT;

CREATE UNIQUE INDEX "wl_invoicing_supplier_bill_allocations_key"
    ON "wl_invoicing_supplier_bill_allocations" (key);
