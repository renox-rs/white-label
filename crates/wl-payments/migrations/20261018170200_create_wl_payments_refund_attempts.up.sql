-- Each try to pay a refund is kept as an attempt, with its failure. Rows
-- are never deleted or rewritten after they fail.
CREATE TABLE "wl_payments_refund_attempts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    refund_id INTEGER NOT NULL REFERENCES "wl_payments_refunds" (id),
    -- 1, 2, ... per refund.
    number INTEGER NOT NULL,
    method TEXT NOT NULL,
    reference TEXT,
    -- The gateway's refund reference; unique when set.
    gateway_ref TEXT,
    -- The outgoing payment `pay` recorded.
    payment_id INTEGER REFERENCES "wl_payments_payments" (id),
    started_on TEXT NOT NULL,
    -- `pending`, `paid` or `failed`.
    outcome TEXT NOT NULL DEFAULT 'pending',
    failure_kind TEXT,
    failure_reason TEXT,
    failed_on TEXT,
    failed_by INTEGER,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (refund_id, number),
    UNIQUE (gateway_ref)
);

CREATE INDEX "wl_payments_refund_attempts_outcome" ON "wl_payments_refund_attempts" (outcome);

-- What the refund comes from, by kind (`payment`, `credit_note`, ...): the
-- source's subject type alone cannot tell an order from a booking.
ALTER TABLE "wl_payments_refunds" ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'payment';
UPDATE "wl_payments_refunds" SET source_kind = 'party_credit' WHERE source_type = 'parties.party';
-- The gateway transaction a gateway refund goes back to.
ALTER TABLE "wl_payments_refunds" ADD COLUMN gateway_txn TEXT;
-- Why the latest attempt failed.
ALTER TABLE "wl_payments_refunds" ADD COLUMN failure_kind TEXT;
ALTER TABLE "wl_payments_refunds" ADD COLUMN failure_reason TEXT;
